//! WASAPI 루프백 캡처.
//!
//! 출력장치(스피커)로 나가는 최종 믹스를 그대로 잡는다 — 창의 상태(최소화/백그라운드)와
//! 무관하다. `autoconvert: true` 로 장치 native 포맷을 48kHz·스테레오·f32 로 자동 변환하므로
//! 캡처 결과를 그대로 songbird RawAdapter(f32 인터리브)에 흘릴 수 있다.

use std::collections::VecDeque;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Result};
use wasapi::{Device, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: u32 = 2;

/// 지연 상한 ≈ 0.5초 분량. 넘으면 가장 오래된 바이트를 버린다.
const MAX_BYTES: usize = (SAMPLE_RATE as usize) * (CHANNELS as usize) * 4 / 2;

struct Shared {
    buf: Mutex<VecDeque<u8>>,
    cv: Condvar,
    closed: AtomicBool,
    captured: AtomicU64,
    dropped: AtomicU64,
    error: Mutex<Option<String>>,
}

impl Shared {
    fn new() -> Self {
        Shared {
            buf: Mutex::new(VecDeque::with_capacity(MAX_BYTES)),
            cv: Condvar::new(),
            closed: AtomicBool::new(false),
            captured: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            error: Mutex::new(None),
        }
    }
}

/// songbird 로 넘길 라이브 PCM 리더. 큐가 비면 무음을 돌려줘 트랙이 끊기지 않게 한다.
pub struct CaptureReader {
    shared: Arc<Shared>,
}

impl Read for CaptureReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let mut buf = self.shared.buf.lock().unwrap();
        loop {
            if self.shared.closed.load(Ordering::Acquire) {
                return Ok(0); // EOF → 트랙 종료
            }
            if !buf.is_empty() {
                let n = out.len().min(buf.len());
                for (i, b) in buf.drain(..n).enumerate() {
                    out[i] = b;
                }
                return Ok(n);
            }
            // 언더런: 잠깐 기다렸다가 그래도 없으면 무음으로 채워 실시간 페이싱 유지
            let (g, res) = self
                .shared
                .cv
                .wait_timeout(buf, Duration::from_millis(20))
                .unwrap();
            buf = g;
            if res.timed_out() && buf.is_empty() {
                for b in out.iter_mut() {
                    *b = 0;
                }
                return Ok(out.len());
            }
        }
    }
}

/// 캡처 스레드 제어 핸들. stop() 으로 종료(리더는 EOF 를 받아 트랙이 끝난다).
pub struct CaptureHandle {
    shared: Arc<Shared>,
    join: Option<thread::JoinHandle<()>>,
    device_label: String,
}

impl CaptureHandle {
    pub fn stop(&mut self) {
        self.shared.closed.store(true, Ordering::Release);
        self.shared.cv.notify_all();
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
    pub fn device_label(&self) -> &str {
        &self.device_label
    }
}

/// 캡처 시작. WASAPI 초기화가 끝날 때까지 기다렸다가 (핸들, 리더) 를 돌려준다.
pub fn start(device_name: Option<String>) -> Result<(CaptureHandle, CaptureReader)> {
    let shared = Arc::new(Shared::new());
    let label = device_name.clone().unwrap_or_else(|| "기본 스피커(시스템 전체 믹스)".to_string());
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let sh = shared.clone();
    let join = thread::Builder::new()
        .name("pepsi-capture".into())
        .spawn(move || {
            if let Err(e) = run_capture(&sh, device_name, &tx) {
                let msg = e.to_string();
                *sh.error.lock().unwrap() = Some(msg.clone());
                let _ = tx.send(Err(msg)); // 초기화 전 실패 시 통보(이미 성공 통보했으면 무시됨)
            }
        })?;

    match rx.recv() {
        Ok(Ok(())) => {
            let handle = CaptureHandle {
                shared: shared.clone(),
                join: Some(join),
                device_label: label,
            };
            let reader = CaptureReader { shared };
            Ok((handle, reader))
        }
        Ok(Err(e)) => Err(anyhow!(e)),
        Err(_) => Err(anyhow!("캡처 스레드가 초기화 전에 종료됨")),
    }
}

fn run_capture(
    shared: &Arc<Shared>,
    device_name: Option<String>,
    tx: &Sender<Result<(), String>>,
) -> Result<()> {
    let _ = wasapi::initialize_mta();

    let device = match device_name {
        Some(ref name) => find_render_device(name)?,
        None => DeviceEnumerator::new()?.get_default_device(&Direction::Render)?,
    };
    let mut audio_client = device.get_iaudioclient()?;

    let format = WaveFormat::new(
        32,
        32,
        &SampleType::Float,
        SAMPLE_RATE as usize,
        CHANNELS as usize,
        None,
    );
    let (_def_time, min_time) = audio_client.get_device_period()?;
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: min_time,
    };
    audio_client.initialize_client(&format, &Direction::Capture, &mode)?;

    let event = audio_client.set_get_eventhandle()?;
    let capture_client = audio_client.get_audiocaptureclient()?;
    audio_client.start_stream()?;

    let _ = tx.send(Ok(())); // 초기화 성공 통보

    let mut local: VecDeque<u8> = VecDeque::new();
    while !shared.closed.load(Ordering::Acquire) {
        // 무음 구간엔 이벤트가 안 와 타임아웃될 수 있다 → 종료 플래그만 확인하고 계속
        if event.wait_for_event(200).is_err() {
            continue;
        }
        capture_client.read_from_device_to_deque(&mut local)?;
        if !local.is_empty() {
            push_bytes(shared, &mut local);
        }
    }
    let _ = audio_client.stop_stream();
    Ok(())
}

fn push_bytes(shared: &Arc<Shared>, local: &mut VecDeque<u8>) {
    let incoming = local.len() as u64;
    let mut buf = shared.buf.lock().unwrap();
    buf.extend(local.drain(..));
    shared.captured.fetch_add(incoming, Ordering::Relaxed);
    if buf.len() > MAX_BYTES {
        let excess = buf.len() - MAX_BYTES;
        buf.drain(..excess);
        shared.dropped.fetch_add(excess as u64, Ordering::Relaxed);
    }
    drop(buf);
    shared.cv.notify_one();
}

fn find_render_device(name: &str) -> Result<Device> {
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
        out.push(dev.get_friendlyname().unwrap_or_else(|_| format!("device {i}")));
    }
    Ok(out)
}

/// 시스템 기본 출력장치 이름.
pub fn default_render_name() -> Result<String> {
    let _ = wasapi::initialize_mta();
    let dev = DeviceEnumerator::new()?.get_default_device(&Direction::Render)?;
    Ok(dev.get_friendlyname()?)
}
