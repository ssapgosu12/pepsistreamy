//! serenity 슬래시 명령 봇 + songbird 음성 송출.
//!
//! 설정은 매번 setting.ini(Settings::load) 를 새로 읽어 /join·/reload 에 즉시 반영된다.
//! TUI 실행 화면에서 상태/볼륨을 보여줄 수 있도록 연결상태·청취자수·상태문자열을 전역으로 노출한다.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use serenity::all::*;
use serenity::async_trait;
use songbird::SerenityInit;
use songbird::input::RawAdapter;
use songbird::input::core::io::ReadOnlySource;

use crate::capture::{self, CaptureHandle, CaptureReader, CaptureSource};
use crate::dsp::DspChain;
use crate::monitor::MonitorSpec;
use crate::settings::Settings;

static CAPTURE: OnceLock<Mutex<Option<CaptureHandle>>> = OnceLock::new();
static PENDING_READER: Mutex<Option<CaptureReader>> = Mutex::new(None);
static CONNECTED: AtomicBool = AtomicBool::new(false);
static LISTENERS: AtomicUsize = AtomicUsize::new(0);
static QUIET: AtomicBool = AtomicBool::new(false);
static STATUS: Mutex<String> = Mutex::new(String::new());
static LOG_GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();

fn slot() -> &'static Mutex<Option<CaptureHandle>> {
    CAPTURE.get_or_init(|| Mutex::new(None))
}

/// songbird/serenity 진단 로그를 pepsistreamy.log 파일로(한 번만 설정).
pub fn init_logging() {
    if LOG_GUARD.get().is_some() {
        return;
    }
    let appender = tracing_appender::rolling::never(".", "pepsistreamy.log");
    let (nb, guard) = tracing_appender::non_blocking(appender);
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new("warn,songbird=debug,serenity=info")
    });
    let _ = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(nb)
        .with_env_filter(filter)
        .try_init();
    let _ = LOG_GUARD.set(guard);
}

/// 실행 화면 진입 시 캡처를 미리 시작(볼륨미터가 /join 전에도 동작하도록). 리더는 보관.
pub fn prestart_capture() {
    if slot().lock().unwrap().is_some() {
        return; // 이미 떠 있음
    }
    match build_config() {
        Ok((source, dsp, monitor)) => match capture::start(source, dsp, monitor) {
            Ok((handle, reader)) => {
                *slot().lock().unwrap() = Some(handle);
                *PENDING_READER.lock().unwrap() = Some(reader);
            }
            Err(e) => set_status(format!("미리 캡처 시작 실패: {e}")),
        },
        Err(e) => set_status(format!("설정 오류: {e}")),
    }
}

/// 음성만 끊고 캡처는 유지(미터 계속). 종료/화면닫기에서는 stop_capture 사용.
async fn disconnect(ctx: &Context, guild_id: GuildId) {
    CONNECTED.store(false, Ordering::Relaxed);
    LISTENERS.store(0, Ordering::Relaxed);
    if let Some(m) = songbird::get(ctx).await {
        let _ = m.remove(guild_id).await;
    }
}

// ---- TUI 가 읽는 전역 상태 ----
pub fn set_quiet(q: bool) {
    QUIET.store(q, Ordering::Relaxed);
}
pub fn is_connected() -> bool {
    CONNECTED.load(Ordering::Relaxed)
}
pub fn listeners() -> usize {
    LISTENERS.load(Ordering::Relaxed)
}
pub fn status() -> String {
    STATUS.lock().unwrap().clone()
}
/// (소스 라벨, 캡처 오류) — 실행 화면 표시용.
pub fn capture_info() -> Option<(String, Option<String>)> {
    slot()
        .lock()
        .unwrap()
        .as_ref()
        .map(|h| (h.source_label().to_string(), h.error()))
}

fn set_status(msg: impl Into<String>) {
    let msg = msg.into();
    if !QUIET.load(Ordering::Relaxed) {
        println!("[pepsistreamy] {msg}");
    }
    *STATUS.lock().unwrap() = msg;
}

/// 캡처 완전 중지 + 상태 초기화(실행 화면 닫을 때).
pub fn stop_capture() {
    if let Some(mut h) = slot().lock().unwrap().take() {
        h.stop();
    }
    *PENDING_READER.lock().unwrap() = None;
    CONNECTED.store(false, Ordering::Relaxed);
    LISTENERS.store(0, Ordering::Relaxed);
}

struct Handler {
    guild_id: Option<u64>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        let cmds = vec![
            CreateCommand::new("join").description("내가 있는 음성 채널로 들어가 방송 시작"),
            CreateCommand::new("leave").description("방송 종료 후 음성 채널에서 나가기"),
            CreateCommand::new("reload").description("setting.ini 의 소스/DSP/모니터를 다시 적용"),
            CreateCommand::new("status").description("캡처/연결 상태 보기"),
        ];
        let res = if let Some(gid) = self.guild_id {
            GuildId::new(gid)
                .set_commands(&ctx.http, cmds)
                .await
                .map(|_| ())
        } else {
            Command::set_global_commands(&ctx.http, cmds)
                .await
                .map(|_| ())
        };
        if let Err(e) = res {
            eprintln!("[pepsistreamy] 슬래시 명령 등록 실패: {e}");
        }
        set_status(format!(
            "로그인됨: {} (서버 {}개). 음성채널 입장 후 /join",
            ready.user.name,
            ready.guilds.len()
        ));
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        let Interaction::Command(command) = interaction else {
            return;
        };
        let content = match command.data.name.as_str() {
            "join" => handle_join(&ctx, &command).await,
            "leave" => handle_leave(&ctx, &command).await,
            "reload" => handle_reload(&ctx, &command).await,
            "status" => handle_status(&ctx, &command).await,
            other => format!("알 수 없는 명령: {other}"),
        };
        let msg = CreateInteractionResponseMessage::new()
            .ephemeral(true)
            .content(content);
        if let Err(e) = command
            .create_response(&ctx.http, CreateInteractionResponse::Message(msg))
            .await
        {
            eprintln!("[pepsistreamy] 응답 실패: {e}");
        }
    }

    /// 음성 상태 변경마다 청취자 수 갱신 + 봇 혼자면 자동 퇴장.
    async fn voice_state_update(&self, ctx: Context, _old: Option<VoiceState>, new: VoiceState) {
        let Some(guild_id) = new.guild_id else {
            return;
        };
        let bot_id = ctx.cache.current_user().id;
        let count = {
            let Some(guild) = ctx.cache.guild(guild_id) else {
                return;
            };
            let Some(bot_ch) = guild.voice_states.get(&bot_id).and_then(|vs| vs.channel_id) else {
                LISTENERS.store(0, Ordering::Relaxed);
                return; // 봇이 음성채널에 없음
            };
            guild
                .voice_states
                .values()
                .filter(|vs| vs.channel_id == Some(bot_ch) && vs.user_id != bot_id)
                .count()
        };
        LISTENERS.store(count, Ordering::Relaxed);
        if count == 0 && is_connected() {
            disconnect(&ctx, guild_id).await; // 캡처는 유지(미터 계속)
            set_status("채널에 아무도 없어 자동 퇴장했습니다.");
        }
    }
}

fn env_set(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .is_some()
}

/// setting.ini(또는 env) 를 새로 읽어 (소스, DSP, 모니터) 구성.
pub(crate) fn build_config()
-> anyhow::Result<(CaptureSource, Option<DspChain>, Option<MonitorSpec>)> {
    dotenvy::dotenv().ok();
    let s = Settings::load();
    let sr = capture::SAMPLE_RATE as f32;
    let source = if env_set("YTCAST_PROCESS") || env_set("YTCAST_DEVICE") {
        CaptureSource::from_env()?
    } else {
        s.capture_source()?
    };
    let dsp = if env_set("YTCAST_DSP") {
        DspChain::from_env(sr)
    } else {
        s.dsp_chain(sr)
    };
    let monitor = s.monitor.then(|| MonitorSpec {
        device: s.monitor_device.clone(),
    });
    Ok((source, dsp, monitor))
}

/// 새 설정으로 캡처를 시작해 슬롯에 저장하고 리더 반환.
fn start_and_store() -> Result<CaptureReader, String> {
    let (source, dsp, monitor) = build_config().map_err(|e| format!("설정 오류: {e}"))?;
    let (handle, reader) =
        capture::start(source, dsp, monitor).map_err(|e| format!("캡처 시작 실패: {e}"))?;
    let mut g = slot().lock().unwrap();
    if let Some(mut old) = g.take() {
        old.stop();
    }
    *g = Some(handle);
    Ok(reader)
}

/// 캡처 리더를 Call 에 송출. restart=true 면 새 설정으로 캡처를 재시작(/reload),
/// false 면 미리 시작된 캡처의 리더를 재사용(/join).
async fn apply_capture(
    call: &std::sync::Arc<serenity::prelude::Mutex<songbird::Call>>,
    restart: bool,
) -> Result<(), String> {
    let reader = if restart {
        *PENDING_READER.lock().unwrap() = None;
        start_and_store()?
    } else if let Some(r) = PENDING_READER.lock().unwrap().take() {
        r
    } else if let Some(r) = slot().lock().unwrap().as_ref().map(|h| h.new_reader()) {
        r // 캡처는 떠 있는데 리더만 소비됨(재-/join) → 새 리더
    } else {
        start_and_store()?
    };
    let src = RawAdapter::new(
        ReadOnlySource::new(reader),
        capture::SAMPLE_RATE,
        capture::CHANNELS,
    );
    let mut h = call.lock().await;
    h.stop();
    h.play_input(src.into());
    Ok(())
}

async fn handle_join(ctx: &Context, command: &CommandInteraction) -> String {
    let Some(guild_id) = command.guild_id else {
        return "서버(길드) 안에서만 사용할 수 있습니다.".to_string();
    };
    let channel_id = {
        let Some(guild) = ctx.cache.guild(guild_id) else {
            return "길드 캐시를 아직 못 읽었습니다. 잠시 후 다시 시도하세요.".to_string();
        };
        guild
            .voice_states
            .get(&command.user.id)
            .and_then(|vs| vs.channel_id)
    };
    let Some(channel_id) = channel_id else {
        return "먼저 음성 채널에 들어간 다음 다시 `/join` 하세요.".to_string();
    };

    let manager = match songbird::get(ctx).await {
        Some(m) => m.clone(),
        None => return "songbird 초기화 안 됨".to_string(),
    };
    let call = match manager.join(guild_id, channel_id).await {
        Ok(c) => c,
        Err(e) => return format!("음성 채널 입장 실패: {e}"),
    };
    if let Err(e) = apply_capture(&call, false).await {
        let _ = manager.remove(guild_id).await;
        return e;
    }
    CONNECTED.store(true, Ordering::Relaxed);
    set_status("방송 시작");
    "▶️ 방송 시작. 브라우저/앱에서 소리를 재생하세요.".to_string()
}

async fn handle_reload(ctx: &Context, command: &CommandInteraction) -> String {
    let Some(guild_id) = command.guild_id else {
        return "서버 안에서만 사용할 수 있습니다.".to_string();
    };
    let manager = match songbird::get(ctx).await {
        Some(m) => m.clone(),
        None => return "songbird 초기화 안 됨".to_string(),
    };
    let call = match manager.get(guild_id) {
        Some(c) => c,
        None => return "연결돼 있지 않습니다. 먼저 `/join` 하세요.".to_string(),
    };
    match apply_capture(&call, true).await {
        Ok(_) => "🔄 설정을 다시 적용했습니다(소스/DSP/모니터).".to_string(),
        Err(e) => e,
    }
}

async fn handle_leave(ctx: &Context, command: &CommandInteraction) -> String {
    let Some(guild_id) = command.guild_id else {
        return "서버 안에서만 사용할 수 있습니다.".to_string();
    };
    disconnect(ctx, guild_id).await; // 캡처는 유지(미터 계속), 음성만 끊음
    set_status("방송 종료, 채널에서 나감");
    "⏹️ 방송 종료, 채널에서 나갔습니다.".to_string()
}

async fn handle_status(_ctx: &Context, _command: &CommandInteraction) -> String {
    let g = slot().lock().unwrap();
    match g.as_ref() {
        Some(h) => {
            let err = h
                .error()
                .map(|e| format!(" (오류: {e})"))
                .unwrap_or_default();
            format!(
                "```\n캡처: 동작중{err}\n연결: {}\n청취자: {}명\n소스: {}\nDSP: {}\n바이트: 캡처 {} / 드롭 {} / 버퍼 {}\n```",
                if is_connected() {
                    "연결됨"
                } else {
                    "미연결"
                },
                listeners(),
                h.source_label(),
                h.dsp_label().unwrap_or("off"),
                h.captured_bytes(),
                h.dropped_bytes(),
                h.buffered_bytes(),
            )
        }
        None => format!(
            "```\n캡처: 중지\n연결: {}\n```",
            if is_connected() {
                "연결됨"
            } else {
                "미연결"
            }
        ),
    }
}

/// setting.ini(또는 env) 에서 토큰·길드ID 가져오기.
pub fn token_and_guild() -> anyhow::Result<(String, Option<u64>)> {
    dotenvy::dotenv().ok();
    let s = Settings::load();
    let token = s
        .token()
        .or_else(|| {
            std::env::var("DISCORD_TOKEN")
                .ok()
                .filter(|t| !t.trim().is_empty())
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "토큰이 없습니다. 인자 없이 `pepsistreamy` 를 실행해 설정 TUI 를 여세요."
            )
        })?;
    let guild_id = s.guild_id.or_else(|| {
        std::env::var("DISCORD_GUILD_ID")
            .ok()
            .and_then(|v| v.trim().parse().ok())
    });
    Ok((token, guild_id))
}

/// serenity 클라이언트 빌드(시작은 호출자가). TUI 실행 화면에서 사용.
pub async fn build_client(token: &str, guild_id: Option<u64>) -> anyhow::Result<Client> {
    let intents = GatewayIntents::GUILDS | GatewayIntents::GUILD_VOICE_STATES;
    let client = Client::builder(token, intents)
        .event_handler(Handler { guild_id })
        .register_songbird()
        .await?;
    Ok(client)
}

/// 콘솔 모드 실행(`run` 명령). Ctrl+C 로 종료.
pub async fn run() -> anyhow::Result<()> {
    set_quiet(false);
    init_logging();
    let (token, guild_id) = token_and_guild()?;
    let mut client = build_client(&token, guild_id).await?;
    println!("[pepsistreamy] 봇 시작 중... (Ctrl+C 로 종료)");
    tokio::select! {
        r = client.start() => { r?; }
        _ = tokio::signal::ctrl_c() => { println!("\n[pepsistreamy] 종료합니다."); }
    }
    stop_capture();
    Ok(())
}
