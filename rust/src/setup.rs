//! 설정 마법사(`pepsistreamy setup`).
//!
//! - 디스코드 개발자 포털을 새 창으로 열어줌
//! - 봇 토큰을 입력받아 .env 작성
//! - 토큰에서 client_id 를 디코드해 **초대 URL 을 자동 생성·오픈**(스코프/권한 미리 채움)
//! - VB-CABLE 설치 여부 감지 → 없으면 공식 다운로드 페이지 안내(번들 X, 링크만)

use std::io::Write;

use anyhow::Result;

use crate::capture;

const DEV_PORTAL: &str = "https://discord.com/developers/applications";
const VBCABLE_URL: &str = "https://vb-audio.com/Cable/";
// Connect(0x100000) + Speak(0x200000) + View Channels(0x400) = 3146752
const INVITE_PERMS: u64 = 3_146_752;

pub fn run_wizard() -> Result<()> {
    println!("=== PepsiStreamy 설정 마법사 ===\n");
    println!("내 PC 소리를 디스코드 음성 채널로 방송하기 위한 봇을 설정합니다.\n");

    // 1) 봇 만들기 — 개발자 포털 자동 오픈
    println!("[1/4] 디스코드 봇 만들기");
    println!(
        "  개발자 포털을 새 창으로 엽니다. New Application → (이름 입력) → 좌측 Bot → Reset Token → Copy."
    );
    println!("  (특권 인텐트는 켤 필요 없습니다.)");
    if prompt_yes_no("  지금 개발자 포털을 열까요?", true) {
        open_url(DEV_PORTAL);
    } else {
        println!("  직접 열기: {DEV_PORTAL}");
    }

    // 2) 토큰 입력
    println!("\n[2/4] 봇 토큰 붙여넣기");
    let token = loop {
        let t = prompt("  봇 토큰: ");
        let t = t.trim().trim_start_matches("Bot ").trim().to_string();
        if t.is_empty() {
            println!("  (비어 있습니다. 다시 입력하거나 Ctrl+C 로 취소)");
            continue;
        }
        break t;
    };
    let guild = prompt("  서버(길드) ID — 명령 즉시 표시용, 선택(엔터로 건너뛰기): ");

    // 3) 서버 초대 — 토큰에서 client_id 추출해 초대 URL 자동 생성
    println!("\n[3/4] 봇을 내 서버에 초대");
    match client_id_from_token(&token) {
        Some(id) => {
            let url = invite_url(&id);
            println!("  초대 URL을 만들었습니다(권한: 채널 보기/연결/말하기 미리 채움):");
            println!("  {url}");
            if prompt_yes_no("  지금 초대 페이지를 열까요?", true) {
                open_url(&url);
                println!("  → 브라우저에서 내 서버를 선택하고 '승인' 하세요.");
            }
        }
        None => {
            println!(
                "  토큰에서 client_id 를 못 읽었습니다. 개발자 포털 > OAuth2 > URL Generator 에서"
            );
            println!(
                "  스코프 bot + applications.commands, 권한 View Channels/Connect/Speak 로 초대하세요."
            );
        }
    }

    // 4) 특정 앱만 캡처 — VB-CABLE 안내(선택)
    println!("\n[4/4] (선택) 특정 앱만 송출");
    println!("  기본은 시스템 전체 소리입니다. 특정 앱만 보내는 두 가지 방법:");
    println!("   (a) 프로세스 캡처:  .env 에 YTCAST_PROCESS=chrome  — 추가 설치 불필요(권장).");
    println!("   (b) 가상 케이블:    그 앱 출력을 VB-CABLE 로 보내고 YTCAST_DEVICE=CABLE.");
    if vbcable_installed() {
        println!("  VB-CABLE: 설치됨 ✓ (방법 b 사용 가능)");
    } else {
        println!("  VB-CABLE: 미설치. (방법 a 프로세스 캡처를 쓰면 없어도 됩니다.)");
        println!("  ※ VB-CABLE 은 VB-Audio 의 도네이션웨어입니다(출처: vb-audio.com).");
        if prompt_yes_no("  VB-CABLE 공식 다운로드 페이지를 열까요?", false) {
            open_url(VBCABLE_URL);
            println!(
                "  → 압축 풀고 VBCABLE_Setup_x64.exe 를 관리자 권한으로 설치 후 재부팅하세요."
            );
        }
    }

    // .env 작성
    write_env(&token, &guild)?;
    println!("\n완료! 이제:");
    println!("  pepsistreamy.exe doctor   (점검)");
    println!("  pepsistreamy.exe meter    (유튜브 틀고 레벨 확인)");
    println!("  pepsistreamy.exe run      (봇 실행) → 디스코드 음성채널 입장 후 /join");
    Ok(())
}

fn write_env(token: &str, guild: &str) -> Result<()> {
    let path = std::path::Path::new(".env");
    if path.exists() && !prompt_yes_no("\n.env 가 이미 있습니다. 덮어쓸까요?", false) {
        println!(".env 는 그대로 두었습니다. 토큰을 직접 넣으세요: DISCORD_TOKEN={token}");
        return Ok(());
    }
    let guild_line = guild.trim();
    let content = format!(
        "# PepsiStreamy 설정 (setup 마법사 생성). 이 파일은 공개/커밋 금지.\n\
         DISCORD_TOKEN={token}\n\
         DISCORD_GUILD_ID={guild_line}\n\
         \n\
         # 특정 앱만 캡처(둘 중 하나). 비우면 시스템 전체 소리.\n\
         # YTCAST_PROCESS=chrome\n\
         # YTCAST_DEVICE=CABLE\n\
         \n\
         # 내장 필터(앰비언트: HP/LP + reverb). 세부 조정: YTCAST_HP/LP/REVERB/ROOM/GAIN\n\
         # YTCAST_DSP=ambient\n"
    );
    std::fs::write(path, content)?;
    println!("\n.env 작성 완료: {}", path.display());
    Ok(())
}

fn invite_url(client_id: &str) -> String {
    format!(
        "https://discord.com/api/oauth2/authorize?client_id={client_id}&permissions={INVITE_PERMS}&scope=bot%20applications.commands"
    )
}

fn open_url(url: &str) {
    // cmd start 로 기본 브라우저에서 열기 (첫 "" 는 start 의 제목 인자)
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
}

fn vbcable_installed() -> bool {
    capture::list_render_devices()
        .map(|v| v.iter().any(|n| n.to_uppercase().contains("CABLE")))
        .unwrap_or(false)
}

fn prompt(msg: &str) -> String {
    print!("{msg}");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
    s.trim().to_string()
}

fn prompt_yes_no(msg: &str, default_yes: bool) -> bool {
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    let ans = prompt(&format!("{msg} {hint} "));
    match ans.to_lowercase().as_str() {
        "y" | "yes" => true,
        "n" | "no" => false,
        _ => default_yes,
    }
}

/// 봇 토큰의 첫 세그먼트(base64) → 숫자 client_id(=application id).
fn client_id_from_token(token: &str) -> Option<String> {
    let first = token.split('.').next()?;
    let bytes = b64_decode(first)?;
    let s = String::from_utf8(bytes).ok()?;
    if !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()) {
        Some(s)
    } else {
        None
    }
}

/// 표준/URL-safe base64 디코드(패딩 무시).
fn b64_decode(input: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' | b'-' => Some(62),
            b'/' | b'_' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &c in input.as_bytes() {
        if c == b'=' {
            break;
        }
        let v = val(c)?;
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_client_id_from_token() {
        // 잘 알려진 예시 토큰 형식: 첫 세그먼트가 user id(80351110224678912)의 base64
        let token = "ODAzNTExMTAyMjQ2Nzg5MTI.YzGTWg.example_signature_part";
        assert_eq!(
            client_id_from_token(token).as_deref(),
            Some("80351110224678912")
        );
    }

    #[test]
    fn rejects_garbage_token() {
        assert_eq!(client_id_from_token("not-a-token"), None);
    }
}
