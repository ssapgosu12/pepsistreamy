//! 로컬 모니터: DSP 처리된 PCM 을 WASAPI 렌더로 출력장치(스피커)에 재생.
//!
//! 송출자도 청취자와 **같은 필터된 소리**를 듣게 하기 위한 것. 캡처 스레드가 처리된
//! 바이트를 write() 로 넣으면, 전용 렌더 스레드가 출력장치로 흘려보낸다.
//!
//! 주의: 일반 루프백 모드에서 켜면 원본+필터본이 겹친다. 프로세스 캡처 + 그 앱을
//! 볼륨 믹서에서 뮤트(또는 VB-CABLE 라우팅)한 상태에서 쓰면 필터본만 들린다.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Result, anyhow};
use wasapi::{Direction, SampleType, StreamMode, WaveFormat};

use crate::capture::{CHANNELS, SAMPLE_RATE};

const BYTES_PER_SEC: usize = SAMPLE_RATE as usize * CHANNELS as usize * 4;
const MAX_BYTES: usize = BYTES_PER_SEC * 300 / 1000; // 지연 상한 ~300ms

pub struct MonitorSpec {
    pub device: Option<String>, // None = 기본 출력장치
}

struct MonShared {
    buf: Mutex<VecDeque<u8>>,
    closed: AtomicBool,
}

pub struct Monitor {
    shared: Arc<MonShared>,
    join: Option<thread::JoinHandle<()>>,
}

impl Monitor {
    pub fn start(spec: MonitorSpec) -> Result<Monitor> {
        let shared = Arc::new(MonShared {
            buf: Mutex::new(VecDeque::with_capacity(MAX_BYTES)),
            closed: AtomicBool::new(false),
        });
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let sh = shared.clone();
        let join = thread::Builder::new()
            .name("pepsi-monitor".into())
            .spawn(move || {
                if let Err(e) = render_loop(&sh, spec.device, &tx) {
                    let _ = tx.send(Err(e.to_string()));
                }
            })?;
        match rx.recv() {
            Ok(Ok(())) => Ok(Monitor {
                shared,
                join: Some(join),
            }),
            Ok(Err(e)) => Err(anyhow!(e)),
            Err(_) => Err(anyhow!("모니터 스레드가 초기화 전에 종료됨")),
        }
    }

    /// 처리된 PCM(48k f32 스테레오) 바이트를 재생 큐에 넣는다(논블로킹, 지연 상한).
    pub fn write(&self, bytes: &[u8]) {
        let mut buf = self.shared.buf.lock().unwrap();
        buf.extend(bytes.iter().copied());
        if buf.len() > MAX_BYTES {
            let excess = buf.len() - MAX_BYTES;
            buf.drain(..excess);
        }
    }

    pub fn stop(&mut self) {
        self.shared.closed.store(true, Ordering::Release);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn render_loop(
    shared: &Arc<MonShared>,
    device: Option<String>,
    tx: &std::sync::mpsc::Sender<Result<(), String>>,
) -> Result<()> {
    let _ = wasapi::initialize_mta();
    let dev = match device {
        Some(ref name) => crate::capture::find_render_device(name)?,
        None => wasapi::DeviceEnumerator::new()?.get_default_device(&Direction::Render)?,
    };
    let mut ac = dev.get_iaudioclient()?;
    let format = WaveFormat::new(
        32,
        32,
        &SampleType::Float,
        SAMPLE_RATE as usize,
        CHANNELS as usize,
        None,
    );
    let blockalign = format.get_blockalign() as usize;
    let (_d, min_t) = ac.get_device_period()?;
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: min_t,
    };
    ac.initialize_client(&format, &Direction::Render, &mode)?;
    let event = ac.set_get_eventhandle()?;
    let render_client = ac.get_audiorenderclient()?;
    ac.start_stream()?;
    let _ = tx.send(Ok(()));

    while !shared.closed.load(Ordering::Acquire) {
        let avail = ac.get_available_space_in_frames()? as usize;
        if avail > 0 {
            let mut buf = shared.buf.lock().unwrap();
            let have_frames = buf.len() / blockalign;
            let n = avail.min(have_frames);
            if n > 0 {
                render_client.write_to_device_from_deque(n, &mut buf, None)?;
            }
            // 부족분(avail-n)은 채우지 않음 → 그 구간 무음(언더런 보호)
        }
        if event.wait_for_event(200).is_err() {
            continue;
        }
    }
    let _ = ac.stop_stream();
    Ok(())
}
