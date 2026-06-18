//! WASAPI 루프백 캡처 (장치 전체 믹스 또는 특정 프로세스).
//!
//! - 기본/장치: 출력장치(스피커) 루프백 — 시스템 전체 믹스(창 상태 무관).
//! - 프로세스: `new_application_loopback_client(pid, tree)` 로 **특정 앱(과 자식)만** 캡처.
//!
//! `autoconvert: true` 로 48kHz·스테레오·f32 로 자동 변환 → (옵션) DSP → songbird RawAdapter.
//! 믹서 스레드를 막지 않도록 read()는 **논블로킹**이고, 시작 시 프리롤(지터버퍼)을 쌓아
//! 간헐적 끊김을 줄인다.

use std::collections::VecDeque;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Result, anyhow};
use wasapi::{
    AudioClient, Device, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat,
};

use crate::dsp::DspChain;

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: u32 = 2;

const BYTES_PER_SEC: usize = SAMPLE_RATE as usize * CHANNELS as usize * 4; // f32
/// 시작 프리롤(지터버퍼) ≈ 60ms — 쌓일 때까지 무음을 내보내 쿠션 확보.
const PRIME_BYTES: usize = BYTES_PER_SEC * 60 / 1000;
/// 지연 상한 ≈ 250ms. 넘으면 가장 오래된 바이트를 버린다.
const MAX_BYTES: usize = BYTES_PER_SEC * 250 / 1000;

/// 캡처 대상.
pub enum CaptureSource {
    DefaultDevice,
    Device(String),
    Process(u32),
}

impl CaptureSource {
    /// env(YTCAST_PROCESS > YTCAST_DEVICE) 로 소스 결정.
    pub fn from_env() -> Result<CaptureSource> {
        if let Ok(p) = std::env::var("YTCAST_PROCESS") {
            let p = p.trim();
            if !p.is_empty() {
                return Ok(CaptureSource::Process(crate::process::resolve(p)?));
            }
        }
        if let Ok(d) = std::env::var("YTCAST_DEVICE") {
            let d = d.trim();
            if !d.is_empty() {
                return Ok(CaptureSource::Device(d.to_string()));
            }
        }
        Ok(CaptureSource::DefaultDevice)
    }

    fn label(&self) -> String {
        match self {
            CaptureSource::DefaultDevice => "기본 스피커(시스템 전체 믹스)".to_string(),
            CaptureSource::Device(n) => format!("출력장치 '{n}'"),
            CaptureSource::Process(pid) => format!(
                "프로세스 {pid}{}",
                crate::process::name_of(*pid)
                    .map(|n| format!(" ({n})"))
                    .unwrap_or_default()
            ),
        }
    }
}

struct Shared {
    buf: Mutex<VecDeque<u8>>,
    closed: AtomicBool,
    primed: AtomicBool,
    captured: AtomicU64,
    dropped: AtomicU64,
    error: Mutex<Option<String>>,
}

impl Shared {
    fn new() -> Self {
        Shared {
            buf: Mutex::new(VecDeque::with_capacity(MAX_BYTES)),
            closed: AtomicBool::new(false),
            primed: AtomicBool::new(false),
            captured: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            error: Mutex::new(None),
        }
    }
}

/// songbird 로 넘길 라이브 PCM 리더. **논블로킹** — 데이터 없으면 즉시 무음을 돌려줘
/// 믹서 타이밍이 밀리지 않게 한다(끊김 방지).
pub struct CaptureReader {
    shared: Arc<Shared>,
}

impl Read for CaptureReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        if self.shared.closed.load(Ordering::Acquire) {
            return Ok(0); // EOF → 트랙 종료
        }
        let mut buf = self.shared.buf.lock().unwrap();

        // 프리롤이 충분히 쌓이기 전엔 무음(시작 쿠션)
        if !self.shared.primed.load(Ordering::Acquire) {
            if buf.len() >= PRIME_BYTES {
                self.shared.primed.store(true, Ordering::Release);
            } else {
                drop(buf);
                out.fill(0);
                return Ok(out.len());
            }
        }
        if buf.is_empty() {
            drop(buf);
            out.fill(0); // 언더런 → 무음(논블로킹)
            return Ok(out.len());
        }
        let n = out.len().min(buf.len());
        for (i, b) in buf.drain(..n).enumerate() {
            out[i] = b;
        }
        Ok(n)
    }
}

/// 캡처 스레드 제어 핸들.
pub struct CaptureHandle {
    shared: Arc<Shared>,
    join: Option<thread::JoinHandle<()>>,
    source_label: String,
    dsp_label: Option<String>,
}

impl CaptureHandle {
    pub fn stop(&mut self) {
        self.shared.closed.store(true, Ordering::Release);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
    pub fn captured_bytes(&self) -> u64 {
        self.shared.captured.load(Ordering::Relaxed)
    }
    pub fn dropped_bytes(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }
    pub fn buffered_bytes(&self) -> usize {
        self.shared.buf.lock().unwrap().len()
    }
    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().unwrap().clone()
    }
    pub fn source_label(&self) -> &str {
        &self.source_label
    }
    pub fn dsp_label(&self) -> Option<&str> {
        self.dsp_label.as_deref()
    }
}

/// 캡처 시작. WASAPI 초기화가 끝날 때까지 기다렸다 (핸들, 리더) 반환.
pub fn start(
    source: CaptureSource,
    dsp: Option<DspChain>,
    monitor: Option<crate::monitor::MonitorSpec>,
) -> Result<(CaptureHandle, CaptureReader)> {
    let shared = Arc::new(Shared::new());
    let source_label = source.label();
    let dsp_label = dsp.as_ref().map(|d| d.label().to_string());

    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let sh = shared.clone();
    let join = thread::Builder::new()
        .name("pepsi-capture".into())
        .spawn(move || {
            if let Err(e) = run_capture(&sh, source, dsp, monitor, &tx) {
                let msg = e.to_string();
                *sh.error.lock().unwrap() = Some(msg.clone());
                let _ = tx.send(Err(msg));
            }
        })?;

    match rx.recv() {
        Ok(Ok(())) => Ok((
            CaptureHandle {
                shared: shared.clone(),
                join: Some(join),
                source_label,
                dsp_label,
            },
            CaptureReader { shared },
        )),
        Ok(Err(e)) => Err(anyhow!(e)),
        Err(_) => Err(anyhow!("캡처 스레드가 초기화 전에 종료됨")),
    }
}

fn run_capture(
    shared: &Arc<Shared>,
    source: CaptureSource,
    mut dsp: Option<DspChain>,
    monitor_spec: Option<crate::monitor::MonitorSpec>,
    tx: &Sender<Result<(), String>>,
) -> Result<()> {
    let _ = wasapi::initialize_mta();

    let (mut audio_client, buffer_duration_hns): (AudioClient, i64) = match source {
        CaptureSource::Process(pid) => {
            // 프로세스 루프백: get_device_period 가 동작 안 하므로 20ms(=200_000 hns) 고정
            (
                AudioClient::new_application_loopback_client(pid, true)?,
                200_000,
            )
        }
        CaptureSource::Device(name) => {
            let ac = find_render_device(&name)?.get_iaudioclient()?;
            let (_d, min_t) = ac.get_device_period()?;
            (ac, min_t)
        }
        CaptureSource::DefaultDevice => {
            let ac = DeviceEnumerator::new()?
                .get_default_device(&Direction::Render)?
                .get_iaudioclient()?;
            let (_d, min_t) = ac.get_device_period()?;
            (ac, min_t)
        }
    };

    let format = WaveFormat::new(
        32,
        32,
        &SampleType::Float,
        SAMPLE_RATE as usize,
        CHANNELS as usize,
        None,
    );
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns,
    };
    audio_client.initialize_client(&format, &Direction::Capture, &mode)?;

    let event = audio_client.set_get_eventhandle()?;
    let capture_client = audio_client.get_audiocaptureclient()?;
    audio_client.start_stream()?;

    let _ = tx.send(Ok(())); // 초기화 성공 통보

    // 모니터(선택): 처리된 소리를 출력장치로도 재생. 실패해도 캡처는 계속.
    let mut monitor = match monitor_spec {
        Some(spec) => match crate::monitor::Monitor::start(spec) {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!("[pepsistreamy] 모니터 시작 실패(무시): {e}");
                None
            }
        },
        None => None,
    };

    let mut local: VecDeque<u8> = VecDeque::new();
    while !shared.closed.load(Ordering::Acquire) {
        if event.wait_for_event(200).is_err() {
            continue; // 무음 구간 타임아웃 → 종료 플래그만 확인하고 계속
        }
        capture_client.read_from_device_to_deque(&mut local)?;
        if local.is_empty() {
            continue;
        }
        let mut bytes: Vec<u8> = local.drain(..).collect();
        if let Some(dsp) = &mut dsp {
            apply_dsp(dsp, &mut bytes);
        }
        if let Some(m) = &monitor {
            m.write(&bytes);
        }
        push_bytes(shared, &bytes);
    }
    if let Some(m) = &mut monitor {
        m.stop();
    }
    let _ = audio_client.stop_stream();
    Ok(())
}

fn apply_dsp(dsp: &mut DspChain, bytes: &mut [u8]) {
    let mut floats: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    dsp.process(&mut floats);
    for (i, f) in floats.iter().enumerate() {
        bytes[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
    }
}

fn push_bytes(shared: &Arc<Shared>, bytes: &[u8]) {
    let mut buf = shared.buf.lock().unwrap();
    buf.extend(bytes.iter().copied());
    shared
        .captured
        .fetch_add(bytes.len() as u64, Ordering::Relaxed);
    if buf.len() > MAX_BYTES {
        let excess = buf.len() - MAX_BYTES;
        buf.drain(..excess);
        shared.dropped.fetch_add(excess as u64, Ordering::Relaxed);
    }
}

pub(crate) fn find_render_device(name: &str) -> Result<Device> {
    let coll = DeviceEnumerator::new()?.get_device_collection(&Direction::Render)?;
    let n = coll.get_nbr_devices()?;
    let want = name.to_lowercase();
    for i in 0..n {
        let dev = coll.get_device_at_index(i)?;
        if let Ok(fname) = dev.get_friendlyname() {
            if fname.to_lowercase().contains(&want) {
                return Ok(dev);
            }
        }
    }
    Err(anyhow!("'{name}' 와 일치하는 출력장치를 못 찾음"))
}

/// 루프백 캡처 가능한 출력장치(스피커) 이름 목록.
pub fn list_render_devices() -> Result<Vec<String>> {
    let _ = wasapi::initialize_mta();
    let coll = DeviceEnumerator::new()?.get_device_collection(&Direction::Render)?;
    let n = coll.get_nbr_devices()?;
    let mut out = Vec::new();
    for i in 0..n {
        let dev = coll.get_device_at_index(i)?;
        out.push(
            dev.get_friendlyname()
                .unwrap_or_else(|_| format!("device {i}")),
        );
    }
    Ok(out)
}

/// 시스템 기본 출력장치 이름.
pub fn default_render_name() -> Result<String> {
    let _ = wasapi::initialize_mta();
    let dev = DeviceEnumerator::new()?.get_default_device(&Direction::Render)?;
    Ok(dev.get_friendlyname()?)
}
