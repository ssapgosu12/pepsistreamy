//! setting.ini 로드/저장 + 토큰 암호화(Windows DPAPI).
//!
//! 핵심: **토큰만 암호문**(DPAPI — 현재 Windows 계정/PC 에 귀속)으로 저장하고,
//! DSP 파라미터·소스(PID 등)는 **평문**으로 둔다. 그래서 누가 setting.ini 를 공유해도
//! DSP/소스 설정만 쓸 수 있고 **토큰은 다른 계정에선 복호화되지 않는다**.

use std::collections::BTreeMap;

use anyhow::{Result, anyhow};

use crate::b64;
use crate::dsp::DspParams;

pub const FILE: &str = "setting.ini";

#[derive(Clone, PartialEq)]
pub enum SourceSel {
    Default,
    Device(String),
    Process(String), // PID 또는 프로세스명
    Legacy(String),  // VB-CABLE 등 가상 케이블 출력장치
}

pub struct Settings {
    pub token_enc: Option<String>, // base64(DPAPI 암호문)
    pub guild_id: Option<u64>,
    pub source: SourceSel,
    pub dsp_enabled: bool,
    pub dsp: DspParams,
    pub monitor: bool,
    pub monitor_device: Option<String>,
    pub profiles: BTreeMap<String, DspParams>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            token_enc: None,
            guild_id: None,
            source: SourceSel::Default,
            dsp_enabled: false,
            dsp: DspParams::default(),
            monitor: false,
            monitor_device: None,
            profiles: BTreeMap::new(),
        }
    }
}

impl Settings {
    pub fn has_token(&self) -> bool {
        self.token_enc.is_some()
    }

    /// 토큰을 암호화해 저장값으로 세팅.
    pub fn set_token(&mut self, plain: &str) -> Result<()> {
        let cipher = dpapi::protect(plain.as_bytes())?;
        self.token_enc = Some(b64::encode(&cipher));
        Ok(())
    }

    /// 저장된 토큰을 복호화. 다른 계정/PC면 None(복호화 실패).
    pub fn token(&self) -> Option<String> {
        let enc = self.token_enc.as_ref()?;
        let cipher = b64::decode(enc)?;
        let plain = dpapi::unprotect(&cipher).ok()?;
        String::from_utf8(plain).ok()
    }

    pub fn dsp_chain(&self, fs: f32) -> Option<crate::dsp::DspChain> {
        self.dsp_enabled
            .then(|| crate::dsp::DspChain::from_params(fs, &self.dsp))
    }

    pub fn capture_source(&self) -> Result<crate::capture::CaptureSource> {
        use crate::capture::CaptureSource;
        Ok(match &self.source {
            SourceSel::Default => CaptureSource::DefaultDevice,
            SourceSel::Device(n) | SourceSel::Legacy(n) => CaptureSource::Device(n.clone()),
            SourceSel::Process(p) => CaptureSource::Process(crate::process::resolve(p)?),
        })
    }

    // ---- 로드/저장 ----

    pub fn load() -> Settings {
        match std::fs::read_to_string(FILE) {
            Ok(text) => Self::from_ini(&text),
            Err(_) => Settings::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        std::fs::write(FILE, self.to_ini()).map_err(|e| anyhow!("setting.ini 저장 실패: {e}"))
    }

    fn from_ini(text: &str) -> Settings {
        let ini = parse_ini(text);
        let get = |sec: &str, key: &str| ini.get(sec).and_then(|m| m.get(key)).cloned();
        let getu8 = |sec: &str, key: &str, d: u8| {
            get(sec, key)
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(d)
        };
        let def = DspParams::default();

        let kind = get("source", "kind").unwrap_or_else(|| "default".into());
        let value = get("source", "value").unwrap_or_default();
        let source = match kind.trim() {
            "device" => SourceSel::Device(value),
            "process" => SourceSel::Process(value),
            "legacy" => SourceSel::Legacy(if value.is_empty() {
                "CABLE".into()
            } else {
                value
            }),
            _ => SourceSel::Default,
        };

        let mut profiles = BTreeMap::new();
        if let Some(p) = ini.get("profiles") {
            for (name, csv) in p {
                if let Some(dp) = parse_profile(csv) {
                    profiles.insert(name.clone(), dp);
                }
            }
        }

        Settings {
            token_enc: get("auth", "token").filter(|s| !s.trim().is_empty()),
            guild_id: get("auth", "guild_id").and_then(|v| v.trim().parse().ok()),
            source,
            dsp_enabled: get("dsp", "enabled")
                .map(|v| v.trim() == "true")
                .unwrap_or(false),
            dsp: DspParams {
                high_freq: getu8("dsp", "high_freq", def.high_freq),
                high_res: getu8("dsp", "high_res", def.high_res),
                low_freq: getu8("dsp", "low_freq", def.low_freq),
                low_res: getu8("dsp", "low_res", def.low_res),
                room: getu8("dsp", "room", def.room),
                mix: getu8("dsp", "mix", def.mix),
            },
            monitor: get("monitor", "enabled")
                .map(|v| v.trim() == "true")
                .unwrap_or(false),
            monitor_device: get("monitor", "device").filter(|s| !s.trim().is_empty()),
            profiles,
        }
    }

    fn to_ini(&self) -> String {
        let (kind, value) = match &self.source {
            SourceSel::Default => ("default", String::new()),
            SourceSel::Device(n) => ("device", n.clone()),
            SourceSel::Process(p) => ("process", p.clone()),
            SourceSel::Legacy(n) => ("legacy", n.clone()),
        };
        let d = &self.dsp;
        let mut s = String::new();
        s.push_str("# PepsiStreamy 설정. 토큰은 DPAPI 암호문(이 PC/계정 전용)이라 공유해도 복호화 안 됨.\n");
        s.push_str("# DSP/소스는 평문 — 자유롭게 공유 가능.\n\n");
        s.push_str("[auth]\n");
        s.push_str(&format!(
            "token = {}\n",
            self.token_enc.clone().unwrap_or_default()
        ));
        s.push_str(&format!(
            "guild_id = {}\n\n",
            self.guild_id.map(|g| g.to_string()).unwrap_or_default()
        ));
        s.push_str("[source]\n");
        s.push_str(&format!("kind = {kind}\n"));
        s.push_str(&format!("value = {value}\n\n"));
        s.push_str("[dsp]\n");
        s.push_str(&format!("enabled = {}\n", self.dsp_enabled));
        s.push_str(&format!("high_freq = {}\n", d.high_freq));
        s.push_str(&format!("high_res = {}\n", d.high_res));
        s.push_str(&format!("low_freq = {}\n", d.low_freq));
        s.push_str(&format!("low_res = {}\n", d.low_res));
        s.push_str(&format!("room = {}\n", d.room));
        s.push_str(&format!("mix = {}\n\n", d.mix));
        s.push_str("[monitor]\n");
        s.push_str(&format!("enabled = {}\n", self.monitor));
        s.push_str(&format!(
            "device = {}\n\n",
            self.monitor_device.clone().unwrap_or_default()
        ));
        s.push_str("[profiles]\n");
        for (name, p) in &self.profiles {
            s.push_str(&format!(
                "{name} = {},{},{},{},{},{}\n",
                p.high_freq, p.high_res, p.low_freq, p.low_res, p.room, p.mix
            ));
        }
        s
    }
}

fn parse_profile(csv: &str) -> Option<DspParams> {
    let v: Vec<u8> = csv
        .split(',')
        .filter_map(|x| x.trim().parse().ok())
        .collect();
    if v.len() == 6 {
        Some(DspParams {
            high_freq: v[0],
            high_res: v[1],
            low_freq: v[2],
            low_res: v[3],
            room: v[4],
            mix: v[5],
        })
    } else {
        None
    }
}

type Ini = BTreeMap<String, BTreeMap<String, String>>;

fn parse_ini(text: &str) -> Ini {
    let mut ini: Ini = BTreeMap::new();
    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_string();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            ini.entry(section.clone())
                .or_default()
                .insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    ini
}

/// Windows DPAPI (현재 사용자 계정에 귀속되는 암호화).
mod dpapi {
    use std::ptr;

    use anyhow::{Result, anyhow};
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CryptProtectData, CryptUnprotectData,
    };

    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>> {
        unsafe {
            let input = CRYPT_INTEGER_BLOB {
                cbData: plain.len() as u32,
                pbData: plain.as_ptr() as *mut u8,
            };
            let mut out = CRYPT_INTEGER_BLOB {
                cbData: 0,
                pbData: ptr::null_mut(),
            };
            let ok = CryptProtectData(
                &input,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            );
            if ok == 0 {
                return Err(anyhow!("CryptProtectData 실패"));
            }
            Ok(std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec())
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn dpapi_roundtrip() {
            let secret = b"my.discord.token-secret";
            let enc = super::protect(secret).unwrap();
            assert_ne!(enc, secret); // 암호문은 평문과 다름
            let dec = super::unprotect(&enc).unwrap();
            assert_eq!(dec, secret);
        }
    }

    pub fn unprotect(cipher: &[u8]) -> Result<Vec<u8>> {
        unsafe {
            let input = CRYPT_INTEGER_BLOB {
                cbData: cipher.len() as u32,
                pbData: cipher.as_ptr() as *mut u8,
            };
            let mut out = CRYPT_INTEGER_BLOB {
                cbData: 0,
                pbData: ptr::null_mut(),
            };
            let ok = CryptUnprotectData(
                &input,
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            );
            if ok == 0 {
                return Err(anyhow!("CryptUnprotectData 실패(다른 계정/PC?)"));
            }
            Ok(std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ini_roundtrip() {
        let mut s = Settings::default();
        s.guild_id = Some(123);
        s.source = SourceSel::Process("chrome".into());
        s.dsp_enabled = true;
        s.dsp.low_freq = 42;
        s.monitor = true;
        s.monitor_device = Some("CABLE".into());
        s.profiles.insert(
            "chill".into(),
            DspParams {
                high_freq: 5,
                high_res: 30,
                low_freq: 40,
                low_res: 20,
                room: 80,
                mix: 40,
            },
        );
        let back = Settings::from_ini(&s.to_ini());
        assert_eq!(back.guild_id, Some(123));
        assert!(matches!(back.source, SourceSel::Process(ref p) if p == "chrome"));
        assert!(back.dsp_enabled);
        assert_eq!(back.dsp.low_freq, 42);
        assert!(back.monitor);
        assert_eq!(back.monitor_device.as_deref(), Some("CABLE"));
        assert_eq!(back.profiles.get("chill").map(|p| p.room), Some(80));
    }
}
