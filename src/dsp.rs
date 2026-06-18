//! 내장 DSP 체인: highpass/lowpass(biquad) + reverb(Freeverb) + gain.
//!
//! 목적: 배경 오디오를 "앰비언트"하게 만들고 **사람 목소리 대역(대략 300–3400Hz)** 과
//! 덜 겹치게 해서 게임/대화에 집중하기 좋게. 캡처(48k 스테레오 f32 인터리브) 직후,
//! songbird 로 보내기 전에 in-place 로 처리한다.
//!
//! 설정(env):
//!   YTCAST_DSP     off(기본) | on | ambient   — 켜기/프리셋
//!   YTCAST_HP      highpass 컷오프 Hz (0=off)
//!   YTCAST_LP      lowpass  컷오프 Hz (0=off)
//!   YTCAST_REVERB  리버브 wet 0.0~1.0 (0=off)
//!   YTCAST_ROOM    리버브 룸사이즈 0.0~1.0
//!   YTCAST_GAIN    최종 게인(0.0~1.5, 배경으로 깔려면 <1)

use std::f32::consts::PI;

/// DSP 파라미터(각 0~100 노브값). TUI/프로필이 이 값을 다룬다.
#[derive(Clone, Copy, PartialEq)]
pub struct DspParams {
    pub high_freq: u8, // highpass 컷오프
    pub high_res: u8,  // highpass Q(레조넌스)
    pub low_freq: u8,  // lowpass 컷오프
    pub low_res: u8,   // lowpass Q
    pub room: u8,      // 리버브 룸사이즈
    pub mix: u8,       // 리버브 wet(섞임)
}

impl Default for DspParams {
    fn default() -> Self {
        DspParams {
            high_freq: 10,
            high_res: 40,
            low_freq: 53,
            low_res: 10,
            room: 60,
            mix: 25,
        }
    }
}

impl DspParams {
    // 0~100 → 실제 값 매핑 (주파수는 로그 스케일)
    pub fn hp_hz(&self) -> f32 {
        20.0 * 100f32.powf(self.high_freq as f32 / 100.0) // 20Hz~2kHz
    }
    pub fn lp_hz(&self) -> f32 {
        200.0 * 90f32.powf(self.low_freq as f32 / 100.0) // 200Hz~18kHz
    }
    fn map_q(v: u8) -> f32 {
        0.5 + (v as f32 / 100.0) * 5.5 // 0.5 ~ 6.0
    }
    pub fn hp_q(&self) -> f32 {
        Self::map_q(self.high_res)
    }
    pub fn lp_q(&self) -> f32 {
        Self::map_q(self.low_res)
    }
    pub fn room01(&self) -> f32 {
        self.room as f32 / 100.0
    }
    pub fn mix01(&self) -> f32 {
        self.mix as f32 / 100.0
    }
}

#[derive(Clone, Copy)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn from_coeffs(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        Biquad {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    pub fn lowpass(fs: f32, fc: f32, q: f32) -> Self {
        let w0 = 2.0 * PI * fc / fs;
        let (sw, cw) = w0.sin_cos();
        let alpha = sw / (2.0 * q);
        Biquad::from_coeffs(
            (1.0 - cw) / 2.0,
            1.0 - cw,
            (1.0 - cw) / 2.0,
            1.0 + alpha,
            -2.0 * cw,
            1.0 - alpha,
        )
    }

    pub fn highpass(fs: f32, fc: f32, q: f32) -> Self {
        let w0 = 2.0 * PI * fc / fs;
        let (sw, cw) = w0.sin_cos();
        let alpha = sw / (2.0 * q);
        Biquad::from_coeffs(
            (1.0 + cw) / 2.0,
            -(1.0 + cw),
            (1.0 + cw) / 2.0,
            1.0 + alpha,
            -2.0 * cw,
            1.0 - alpha,
        )
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

// ---- Freeverb (comb + allpass) ----

struct Comb {
    buf: Vec<f32>,
    idx: usize,
    store: f32,
    feedback: f32,
    damp1: f32,
    damp2: f32,
}

impl Comb {
    fn new(len: usize, feedback: f32, damp: f32) -> Self {
        Comb {
            buf: vec![0.0; len.max(1)],
            idx: 0,
            store: 0.0,
            feedback,
            damp1: damp,
            damp2: 1.0 - damp,
        }
    }
    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let out = self.buf[self.idx];
        self.store = out * self.damp2 + self.store * self.damp1;
        self.buf[self.idx] = input + self.store * self.feedback;
        self.idx += 1;
        if self.idx >= self.buf.len() {
            self.idx = 0;
        }
        out
    }
}

struct Allpass {
    buf: Vec<f32>,
    idx: usize,
    feedback: f32,
}

impl Allpass {
    fn new(len: usize, feedback: f32) -> Self {
        Allpass {
            buf: vec![0.0; len.max(1)],
            idx: 0,
            feedback,
        }
    }
    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let bufout = self.buf[self.idx];
        let out = -input + bufout;
        self.buf[self.idx] = input + bufout * self.feedback;
        self.idx += 1;
        if self.idx >= self.buf.len() {
            self.idx = 0;
        }
        out
    }
}

// Freeverb 표준 튜닝(44100 기준 샘플수) — 실제 fs 로 스케일.
const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;

struct Reverb {
    combs_l: Vec<Comb>,
    combs_r: Vec<Comb>,
    allp_l: Vec<Allpass>,
    allp_r: Vec<Allpass>,
    wet1: f32,
    wet2: f32,
    dry: f32,
}

impl Reverb {
    fn new(fs: f32, wet: f32, room: f32, damp: f32) -> Self {
        let scale = |n: usize, extra: usize| ((n + extra) as f32 * fs / 44100.0) as usize;
        let feedback = room * 0.28 + 0.7; // roomsize → 피드백
        let mk_combs = |spread: usize| {
            COMB_TUNING
                .iter()
                .map(|&t| Comb::new(scale(t, spread), feedback, damp))
                .collect::<Vec<_>>()
        };
        let mk_allp = |spread: usize| {
            ALLPASS_TUNING
                .iter()
                .map(|&t| Allpass::new(scale(t, spread), 0.5))
                .collect::<Vec<_>>()
        };
        let width = 1.0_f32;
        Reverb {
            combs_l: mk_combs(0),
            combs_r: mk_combs(STEREO_SPREAD),
            allp_l: mk_allp(0),
            allp_r: mk_allp(STEREO_SPREAD),
            wet1: wet * (width / 2.0 + 0.5),
            wet2: wet * ((1.0 - width) / 2.0),
            dry: 1.0 - wet,
        }
    }

    #[inline]
    fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let input = (l + r) * 0.015; // Freeverb fixed input gain
        let mut out_l = 0.0;
        let mut out_r = 0.0;
        for c in &mut self.combs_l {
            out_l += c.process(input);
        }
        for c in &mut self.combs_r {
            out_r += c.process(input);
        }
        for a in &mut self.allp_l {
            out_l = a.process(out_l);
        }
        for a in &mut self.allp_r {
            out_r = a.process(out_r);
        }
        let lo = out_l * self.wet1 + out_r * self.wet2 + l * self.dry;
        let ro = out_r * self.wet1 + out_l * self.wet2 + r * self.dry;
        (lo, ro)
    }
}

pub struct DspChain {
    hp: Option<[Biquad; 2]>,
    lp: Option<[Biquad; 2]>,
    reverb: Option<Reverb>,
    gain: f32,
    label: String,
}

impl DspChain {
    pub fn label(&self) -> &str {
        &self.label
    }

    /// 0~100 노브 파라미터로 체인 구성.
    pub fn from_params(fs: f32, p: &DspParams) -> DspChain {
        let hp_hz = p.hp_hz();
        let hp_q = p.hp_q();
        let hp = (p.high_freq > 0).then(|| {
            [
                Biquad::highpass(fs, hp_hz, hp_q),
                Biquad::highpass(fs, hp_hz, hp_q),
            ]
        });
        let lp_hz = p.lp_hz();
        let lp_q = p.lp_q();
        let lp = (p.low_freq < 100 && lp_hz < fs / 2.0).then(|| {
            [
                Biquad::lowpass(fs, lp_hz, lp_q),
                Biquad::lowpass(fs, lp_hz, lp_q),
            ]
        });
        let mix = p.mix01();
        let reverb = (mix > 0.0).then(|| Reverb::new(fs, mix, p.room01(), 0.5));
        let label = format!(
            "HP {:.0}Hz(Q{:.1}) / LP {:.0}Hz(Q{:.1}) / reverb {:.2}(room {:.2})",
            hp_hz,
            hp_q,
            lp_hz,
            lp_q,
            mix,
            p.room01(),
        );
        DspChain {
            hp,
            lp,
            reverb,
            gain: 1.0,
            label,
        }
    }

    /// 48k 스테레오 f32 인터리브 in-place 처리.
    pub fn process(&mut self, samples: &mut [f32]) {
        for frame in samples.chunks_mut(2) {
            if frame.len() < 2 {
                break;
            }
            let mut l = frame[0];
            let mut r = frame[1];
            if let Some(hp) = &mut self.hp {
                l = hp[0].process(l);
                r = hp[1].process(r);
            }
            if let Some(lp) = &mut self.lp {
                l = lp[0].process(l);
                r = lp[1].process(r);
            }
            if let Some(rv) = &mut self.reverb {
                let (wl, wr) = rv.process(l, r);
                l = wl;
                r = wr;
            }
            frame[0] = (l * self.gain).clamp(-1.0, 1.0);
            frame[1] = (r * self.gain).clamp(-1.0, 1.0);
        }
    }

    /// env 설정으로 체인 구성. 비활성이면 None.
    pub fn from_env(fs: f32) -> Option<DspChain> {
        let mode = std::env::var("YTCAST_DSP")
            .unwrap_or_default()
            .to_lowercase();
        let enabled = matches!(mode.as_str(), "on" | "ambient" | "1" | "true" | "yes");
        if !enabled {
            return None;
        }

        // ambient 프리셋 기본값(목소리 대역 위쪽을 깎아 배경으로)
        let mut hp_hz = 120.0;
        let mut lp_hz = 1000.0;
        let mut wet = 0.35;
        let mut room = 0.7;
        let mut gain = 0.55;

        let getf = |k: &str| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.trim().parse::<f32>().ok())
        };
        if let Some(v) = getf("YTCAST_HP") {
            hp_hz = v;
        }
        if let Some(v) = getf("YTCAST_LP") {
            lp_hz = v;
        }
        if let Some(v) = getf("YTCAST_REVERB") {
            wet = v;
        }
        if let Some(v) = getf("YTCAST_ROOM") {
            room = v;
        }
        if let Some(v) = getf("YTCAST_GAIN") {
            gain = v;
        }

        let q = 0.707; // Butterworth
        let hp = (hp_hz > 0.0).then(|| {
            [
                Biquad::highpass(fs, hp_hz, q),
                Biquad::highpass(fs, hp_hz, q),
            ]
        });
        let lp = (lp_hz > 0.0 && lp_hz < fs / 2.0)
            .then(|| [Biquad::lowpass(fs, lp_hz, q), Biquad::lowpass(fs, lp_hz, q)]);
        let reverb =
            (wet > 0.0).then(|| Reverb::new(fs, wet.clamp(0.0, 1.0), room.clamp(0.0, 1.0), 0.5));

        let label = format!(
            "HP {} / LP {} / reverb {:.2}(room {:.2}) / gain {:.2}",
            if hp_hz > 0.0 {
                format!("{hp_hz:.0}Hz")
            } else {
                "off".into()
            },
            if lp_hz > 0.0 {
                format!("{lp_hz:.0}Hz")
            } else {
                "off".into()
            },
            wet,
            room,
            gain,
        );

        Some(DspChain {
            hp,
            lp,
            reverb,
            gain,
            label,
        })
    }
}
