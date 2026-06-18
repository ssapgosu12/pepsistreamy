//! 디스코드 봇 설정 보조 함수 (개발자 포털/초대 URL/VB-CABLE 감지). TUI 에서 사용.

use crate::b64;

pub const DEV_PORTAL: &str = "https://discord.com/developers/applications";
pub const VBCABLE_URL: &str = "https://vb-audio.com/Cable/";
// Connect(0x100000) + Speak(0x200000) + View Channels(0x400) = 3146752
const INVITE_PERMS: u64 = 3_146_752;

/// 기본 브라우저로 URL 열기. ShellExecuteW 사용 — cmd 를 거치지 않아 URL 안의 `&` 가
/// 명령 구분자로 오해되지 않는다.
pub fn open_url(url: &str) {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    let wide = |s: &str| {
        s.encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    let op = wide("open");
    let file = wide(url);
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1, // SW_SHOWNORMAL
        );
    }
}

pub fn invite_url(client_id: &str) -> String {
    format!(
        "https://discord.com/api/oauth2/authorize?client_id={client_id}&permissions={INVITE_PERMS}&scope=bot%20applications.commands"
    )
}

/// 봇 토큰의 첫 세그먼트(base64) → 숫자 client_id(=application id).
pub fn client_id_from_token(token: &str) -> Option<String> {
    let first = token.split('.').next()?;
    let bytes = b64::decode(first)?;
    let s = String::from_utf8(bytes).ok()?;
    (!s.is_empty() && s.bytes().all(|c| c.is_ascii_digit())).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_client_id_from_token() {
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
