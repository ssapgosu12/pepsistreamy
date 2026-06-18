//! pepsistreamy — 내 PC 시스템 소리(브라우저의 유튜브 등)를 디스코드 음성 채널로 실시간 송출.
//!
//! 서브커맨드:
//!   run       캡처 + 봇 실행 (기본)
//!   devices   캡처 가능한 출력장치 목록
//!   doctor    환경 점검

mod bot;
mod capture;
mod dsp;
mod process;
mod setup;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let arg = std::env::args().nth(1).unwrap_or_else(|| "run".to_string());
    match arg.as_str() {
        "run" => bot::run().await,
        "devices" => {
            cmd_devices();
            Ok(())
        }
        "processes" | "procs" => {
            cmd_processes();
            Ok(())
        }
        "setup" => setup::run_wizard(),
        "doctor" => {
            cmd_doctor();
            Ok(())
        }
        "meter" => {
            let secs = std::env::args()
                .nth(2)
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(10.0);
            cmd_meter(secs);
            Ok(())
        }
        "-h" | "--help" | "help" => {
            print_help();
            Ok(())
        }
        other => {
            eprintln!("알 수 없는 명령: {other}\n");
            print_help();
            std::process::exit(2);
        }
    }
}

fn print_help() {
    println!("pepsistreamy — 내 PC 소리를 디스코드 음성 채널로 방송\n");
    println!("사용법: pepsistreamy <명령>");
    println!("  setup     설정 마법사(봇 토큰 입력·초대링크·VB-CABLE 안내)");
    println!("  run       캡처 + 봇 실행 (기본값)");
    println!("  devices   캡처 가능한 출력장치(스피커) 목록");
    println!("  processes 실행 중 프로세스 목록(특정 앱 캡처용 PID 확인)");
    println!("  meter     선택 소스의 캡처 레벨 확인 (예: meter 10)");
    println!("  doctor    환경 점검");
    println!("\n설정(.env):");
    println!("  DISCORD_TOKEN(필수), DISCORD_GUILD_ID(선택)");
    println!("  YTCAST_PROCESS  특정 앱만 캡처(PID 또는 프로세스명). 비우면 장치 캡처.");
    println!("  YTCAST_DEVICE   특정 출력장치 캡처(이름 일부). 비우면 기본 스피커.");
    println!(
        "  YTCAST_DSP      off|on|ambient  내장 필터(HP/LP+reverb). 세부: YTCAST_HP/LP/REVERB/ROOM/GAIN"
    );
}

fn cmd_devices() {
    match capture::list_render_devices() {
        Ok(list) => {
            let default = capture::default_render_name().unwrap_or_default();
            println!("출력장치(스피커) — 루프백 캡처:");
            for name in list {
                let mark = if name == default {
                    "  <- 시스템 기본"
                } else {
                    ""
                };
                println!("  - {name}{mark}");
            }
            println!("\n특정 장치를 쓰려면 .env 에  YTCAST_DEVICE=장치이름일부  (부분일치).");
            println!("  - 시스템 전체 믹스를 보내려면 비워두세요.");
        }
        Err(e) => eprintln!("장치 조회 실패: {e}"),
    }
}

fn cmd_processes() {
    let list = process::list();
    println!("실행 중 프로세스 (특정 앱만 캡처하려면 .env 에 YTCAST_PROCESS=PID 또는 이름):\n");
    println!("  {:>8}  이름", "PID");
    for (pid, name) in list {
        println!("  {pid:>8}  {name}");
    }
    println!(
        "\n예) 크롬만: YTCAST_PROCESS=chrome  (크롬은 모든 탭을 한 오디오 프로세스에서 섞으므로,\n   단일 탭만 분리하려면 그 탭을 전용 브라우저/프로필에 띄우세요.)"
    );
}

fn cmd_meter(seconds: f64) {
    use std::io::{IsTerminal, Read, Write};

    dotenvy::dotenv().ok();
    let source = match capture::CaptureSource::from_env() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("캡처 소스 결정 실패: {e}");
            return;
        }
    };
    let dsp = dsp::DspChain::from_env(capture::SAMPLE_RATE as f32);
    let (mut handle, mut reader) = match capture::start(source, dsp) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("캡처 시작 실패: {e}");
            return;
        }
    };
    println!(
        "[meter] 소스: {} | DSP: {} — {seconds:.0}초간 측정. 지금 유튜브를 재생해 보세요. (최소화해도 됩니다)",
        handle.source_label(),
        handle.dsp_label().unwrap_or("off"),
    );

    let mut buf =
        vec![0u8; (capture::SAMPLE_RATE as usize) * (capture::CHANNELS as usize) * 4 / 50]; // 20ms
    let start = std::time::Instant::now();
    let mut peak = 0.0f32;
    let is_tty = std::io::stdout().is_terminal();
    let mut last_log = start;
    while start.elapsed().as_secs_f64() < seconds {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        let samples = n / 4;
        let mut sumsq = 0.0f64;
        for i in 0..samples {
            let b = i * 4;
            let v = f32::from_le_bytes([buf[b], buf[b + 1], buf[b + 2], buf[b + 3]]);
            sumsq += (v as f64) * (v as f64);
            peak = peak.max(v.abs());
        }
        let rms = if samples > 0 {
            (sumsq / samples as f64).sqrt()
        } else {
            0.0
        };
        let db = if rms < 1e-6 {
            -90.0
        } else {
            20.0 * rms.log10()
        };
        if is_tty {
            let bars = (((db + 60.0) / 60.0).clamp(0.0, 1.0) * 40.0) as usize;
            print!("\r  [{:<40}] {db:6.1} dBFS ", "#".repeat(bars));
            let _ = std::io::stdout().flush();
        } else if last_log.elapsed().as_secs_f64() >= 1.0 {
            // 비TTY(파이프/로그)에선 \r 스팸 대신 1초마다 한 줄
            println!("  {db:6.1} dBFS");
            last_log = std::time::Instant::now();
        }
    }
    handle.stop();
    println!();
    if let Some(e) = handle.error() {
        eprintln!("[meter] 캡처 오류: {e}");
        return;
    }
    if peak < 1e-4 {
        println!("[meter] 소리가 거의 안 잡혔습니다. 장치 선택/앱 출력 라우팅을 확인하세요.");
    } else {
        println!(
            "[meter] 정상 — 최대 레벨 {:.1} dBFS. 캡처 동작합니다.",
            20.0 * peak.log10()
        );
    }
}

fn cmd_doctor() {
    println!("pepsistreamy 환경 점검");
    dotenvy::dotenv().ok();
    let token = std::env::var("DISCORD_TOKEN")
        .ok()
        .filter(|s| !s.is_empty());
    println!(
        "  [{}] DISCORD_TOKEN 설정됨",
        if token.is_some() { "OK" } else { "X " }
    );
    match capture::default_render_name() {
        Ok(n) => println!("  [OK] 기본 출력장치: {n}"),
        Err(e) => println!("  [X ] 출력장치 조회 실패: {e}"),
    }
    println!("  [OK] 버전: pepsistreamy {}", env!("CARGO_PKG_VERSION"));
    if token.is_none() {
        println!("\n.env 에 DISCORD_TOKEN=... 추가 후  `pepsistreamy run`");
    } else {
        println!("\n준비 완료 —  `pepsistreamy run`");
    }
}
