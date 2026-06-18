//! serenity 슬래시 명령 봇 + songbird 음성 송출.
//!
//! 설정은 setting.ini(Settings) 우선, 환경변수로 덮어쓸 수 있다. 슬래시 명령만 쓰므로
//! 특권 인텐트가 필요 없다.

use std::sync::{Mutex, OnceLock};

use serenity::all::*;
use serenity::async_trait;
use songbird::SerenityInit;
use songbird::input::RawAdapter;
use songbird::input::core::io::ReadOnlySource;

use crate::capture::{self, CaptureHandle, CaptureSource};
use crate::monitor::MonitorSpec;
use crate::settings::Settings;

static CAPTURE: OnceLock<Mutex<Option<CaptureHandle>>> = OnceLock::new();
static SETTINGS: OnceLock<Settings> = OnceLock::new();

fn slot() -> &'static Mutex<Option<CaptureHandle>> {
    CAPTURE.get_or_init(|| Mutex::new(None))
}

fn settings() -> &'static Settings {
    SETTINGS.get_or_init(Settings::load)
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
        println!(
            "[pepsistreamy] 로그인됨: {} (서버 {}개)",
            ready.user.name,
            ready.guilds.len()
        );
        println!("[pepsistreamy] 디스코드에서 음성 채널 입장 후 /join 입력하세요.");
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        let Interaction::Command(command) = interaction else {
            return;
        };
        let content = match command.data.name.as_str() {
            "join" => handle_join(&ctx, &command).await,
            "leave" => handle_leave(&ctx, &command).await,
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
}

fn env_set(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .is_some()
}

fn build_source() -> anyhow::Result<CaptureSource> {
    if env_set("YTCAST_PROCESS") || env_set("YTCAST_DEVICE") {
        CaptureSource::from_env()
    } else {
        settings().capture_source()
    }
}

fn build_dsp() -> Option<crate::dsp::DspChain> {
    if env_set("YTCAST_DSP") {
        crate::dsp::DspChain::from_env(capture::SAMPLE_RATE as f32)
    } else {
        settings().dsp_chain(capture::SAMPLE_RATE as f32)
    }
}

fn build_monitor() -> Option<MonitorSpec> {
    let s = settings();
    s.monitor.then(|| MonitorSpec {
        device: s.monitor_device.clone(),
    })
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

    let source = match build_source() {
        Ok(s) => s,
        Err(e) => return format!("캡처 소스 결정 실패: {e}"),
    };
    let dsp = build_dsp();
    let monitor = build_monitor();
    let (handle, reader) = match capture::start(source, dsp, monitor) {
        Ok(v) => v,
        Err(e) => return format!("캡처 시작 실패: {e}"),
    };
    {
        let mut g = slot().lock().unwrap();
        if let Some(mut old) = g.take() {
            old.stop();
        }
        *g = Some(handle);
    }

    let manager = match songbird::get(ctx).await {
        Some(m) => m.clone(),
        None => return "songbird 초기화 안 됨".to_string(),
    };
    let call = match manager.join(guild_id, channel_id).await {
        Ok(c) => c,
        Err(e) => {
            if let Some(mut h) = slot().lock().unwrap().take() {
                h.stop();
            }
            return format!("음성 채널 입장 실패: {e}");
        }
    };
    let src = RawAdapter::new(
        ReadOnlySource::new(reader),
        capture::SAMPLE_RATE,
        capture::CHANNELS,
    );
    {
        let mut handler = call.lock().await;
        handler.stop();
        handler.play_input(src.into());
    }
    "▶️ 방송 시작. 브라우저/앱에서 소리를 재생하세요.".to_string()
}

async fn handle_leave(ctx: &Context, command: &CommandInteraction) -> String {
    let Some(guild_id) = command.guild_id else {
        return "서버 안에서만 사용할 수 있습니다.".to_string();
    };
    if let Some(mut h) = slot().lock().unwrap().take() {
        h.stop();
    }
    if let Some(manager) = songbird::get(ctx).await {
        let _ = manager.remove(guild_id).await;
    }
    "⏹️ 방송 종료, 채널에서 나갔습니다.".to_string()
}

async fn handle_status(ctx: &Context, command: &CommandInteraction) -> String {
    let connected = match command.guild_id {
        Some(gid) => match songbird::get(ctx).await {
            Some(m) => m.get(gid).is_some(),
            None => false,
        },
        None => false,
    };
    let conn = if connected {
        "음성채널 연결됨"
    } else {
        "미연결"
    };
    let g = slot().lock().unwrap();
    match g.as_ref() {
        Some(h) => {
            let err = h
                .error()
                .map(|e| format!(" (오류: {e})"))
                .unwrap_or_default();
            format!(
                "```\n캡처: 동작중{err}\n연결: {conn}\n소스: {}\nDSP: {}\n바이트: 캡처 {} / 드롭 {} / 버퍼 {}\n```",
                h.source_label(),
                h.dsp_label().unwrap_or("off"),
                h.captured_bytes(),
                h.dropped_bytes(),
                h.buffered_bytes(),
            )
        }
        None => format!("```\n캡처: 중지\n연결: {conn}\n```"),
    }
}

pub async fn run() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let s = settings();
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

    let intents = GatewayIntents::GUILDS | GatewayIntents::GUILD_VOICE_STATES;
    let mut client = Client::builder(&token, intents)
        .event_handler(Handler { guild_id })
        .register_songbird()
        .await?;

    println!("[pepsistreamy] 봇 시작 중... (Ctrl+C 로 종료)");
    tokio::select! {
        r = client.start() => { r?; }
        _ = tokio::signal::ctrl_c() => { println!("\n[pepsistreamy] 종료합니다."); }
    }

    if let Some(mut h) = slot().lock().unwrap().take() {
        h.stop();
    }
    Ok(())
}
