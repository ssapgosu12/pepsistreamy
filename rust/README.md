# PepsiStreamy — 네이티브(Rust) 버전 🦀

[루트 Python 버전](../README.md)과 기능이 같고, **런타임 의존성이 전혀 없는 단일 실행파일(.exe ~16MB)** 로 빌드되는 네이티브 구현입니다. 파이썬 설치가 필요 없어 친구에게는 **exe 하나만** 주면 됩니다. 그리고 Python 버전에 없는 기능이 더 있습니다:

- **WASAPI 루프백** 캡처([`wasapi`](https://crates.io/crates/wasapi)) — 창 최소화/백그라운드 무관, 시스템 출력 믹스.
- **특정 앱만 캡처**(프로세스 루프백) — 가상 케이블 없이 한 앱(과 자식)만. `YTCAST_PROCESS`.
- **내장 DSP** — highpass/lowpass + reverb. 배경음을 앰비언트하게 + 목소리 대역과 덜 겹치게. `YTCAST_DSP`.
- **설정 마법사** `setup` — 봇 토큰 입력·초대링크 자동 생성/오픈·VB-CABLE 안내.
- 송출: [`serenity`](https://crates.io/crates/serenity) + [`songbird`](https://crates.io/crates/songbird) (Opus 인코딩·암호화). 라이브 PCM 리더는 **논블로킹**이라 믹서 타이밍이 밀리지 않습니다(끊김 완화).

## 받아서 바로 쓰기 (빌드 불필요)

[Releases](https://github.com/ssapgosu12/pepsistreamy/releases) 에서 `pepsistreamy.exe` 를 받고:

```powershell
.\pepsistreamy.exe setup    # 마법사: 봇 만들기 페이지 열기 → 토큰 입력 → 초대링크 자동 → .env 작성
.\pepsistreamy.exe doctor   # 점검
.\pepsistreamy.exe meter    # 유튜브 틀고 레벨 확인
.\pepsistreamy.exe run      # 봇 실행 → 디스코드 음성채널 입장 후 /join
```

`setup` 없이 수동으로 하려면 `.env.example` 을 `.env` 로 복사해 `DISCORD_TOKEN` 만 채우면 됩니다.

## 소스에서 빌드

요구: **Rust(rustup)** + **Visual Studio Build Tools(C++ "x64 빌드 도구")** + **CMake**(libopus 빌드용).

```powershell
# Rust 설치(최초 1회): https://rustup.rs  (또는)  winget install Rustlang.Rustup
cd rust
cargo build --release        # 산출물: target\release\pepsistreamy.exe
```

## 명령

| 명령 | 설명 |
|------|------|
| `pepsistreamy setup` | 설정 마법사(토큰 입력·초대링크·VB-CABLE 안내) |
| `pepsistreamy run` | 캡처 + 봇 실행 (기본값) |
| `pepsistreamy devices` | 캡처 가능한 출력장치 목록 |
| `pepsistreamy processes` | 실행 중 프로세스 목록(특정 앱 캡처용 PID 확인) |
| `pepsistreamy meter [초]` | 선택 소스의 캡처 레벨 확인 (기본 10초) |
| `pepsistreamy doctor` | 토큰/기본 장치 점검 |
| 디스코드 `/join` `/leave` `/status` | 음성채널 입장/종료/상태(상태에 소스·DSP 표시) |

## 설정(.env)

| 키 | 설명 |
|----|------|
| `DISCORD_TOKEN` | (필수) 봇 토큰 |
| `DISCORD_GUILD_ID` | 슬래시 명령 즉시 동기화할 서버 ID(비우면 전역, 최대 1시간) |
| `YTCAST_PROCESS` | 특정 앱만 캡처 — PID 또는 프로세스명(부분일치). 추가 설치 불필요 |
| `YTCAST_DEVICE` | 특정 출력장치 캡처 — 이름 일부(가상 케이블 분리용) |
| `YTCAST_DSP` | `off`(기본)·`on`·`ambient` — 내장 필터 |
| `YTCAST_HP` `YTCAST_LP` | highpass/lowpass 컷오프 Hz (0=off) |
| `YTCAST_REVERB` `YTCAST_ROOM` | 리버브 wet / 룸사이즈 (0~1) |
| `YTCAST_GAIN` | 최종 게인(배경으로 깔려면 <1) |

소스 우선순위: **PROCESS > DEVICE > 기본 스피커**. `.env` 는 실행 위치(또는 상위 폴더)에서 자동 탐색.

## 특정 앱만 캡처 (프로세스 루프백)

가상 케이블 없이 한 앱의 소리만 송출할 수 있습니다(WASAPI process loopback).

```powershell
.\pepsistreamy.exe processes        # PID/이름 확인
# .env 에:
#   YTCAST_PROCESS=chrome           (이름 부분일치 → 루트 프로세스 자동 선택)
#   또는 YTCAST_PROCESS=12345       (정확한 PID)
```

대상 프로세스와 **그 자식 프로세스**까지 캡처합니다. 단, **크롬은 모든 탭의 오디오를 하나의 오디오 프로세스에서 섞기** 때문에 크롬을 지정하면 모든 탭이 잡힙니다 — 단일 탭만 분리하려면 그 탭을 **전용 브라우저/프로필**에 띄우고 그 프로세스를 지정하세요.

## 내장 필터 (DSP) — 게임/대화에 안 묻히는 배경음

배경 오디오를 앰비언트하게 만들고 **사람 목소리 대역(대략 300–3400Hz)** 과 덜 겹치게 합니다.

```
YTCAST_DSP=ambient        # 프리셋: HP120Hz, LP1000Hz, reverb0.35, room0.7, gain0.55
```

세부 조정 예:
```
YTCAST_DSP=on
YTCAST_LP=800             # 더 어둡게(목소리 위쪽을 더 깎음)
YTCAST_REVERB=0.5         # 더 넓은 공간감
YTCAST_GAIN=0.4          # 더 작게 깔기
```

원리: 캡처(48k f32) 직후 biquad highpass→lowpass→Freeverb→gain 순으로 처리해 songbird 로 보냅니다. `meter`/`/status` 에 적용된 체인이 표시됩니다.

## 끊김(전송) 안정성

라이브 PCM 리더가 **믹서 스레드를 절대 블로킹하지 않도록**(데이터 없으면 즉시 무음 반환) 만들고, 시작 시 ~60ms **프리롤(지터버퍼)** 을 쌓아 간헐적 끊김을 줄였습니다. 그래도 끊긴다면: 디스코드 음성 채널 **비트레이트를 64–96kbps로** 낮추기, 서버 **음성 지역(Region)** 을 가까운 곳으로, 유선/안정적 네트워크 사용을 권장합니다.

## 구조

```
rust/
├─ Cargo.toml          # 의존성 + 릴리스 프로필(strip, lto)
└─ src/
   ├─ main.rs          # CLI: setup / run / devices / processes / meter / doctor
   ├─ capture.rs       # WASAPI 루프백(장치/프로세스) → 논블로킹 라이브 PCM 리더
   ├─ dsp.rs           # biquad HP/LP + Freeverb + gain
   ├─ process.rs       # 프로세스 열거/이름→PID 해석
   ├─ setup.rs         # 설정 마법사(토큰→초대링크, VB-CABLE 안내)
   └─ bot.rs           # serenity 슬래시 명령 + songbird 송출
```

## VB-CABLE 안내

특정 앱 분리는 위 **프로세스 캡처**(추가 설치 불필요)를 권장합니다. 대안으로 가상 케이블을 쓰려면 [VB-CABLE](https://vb-audio.com/Cable/)(VB-Audio의 도네이션웨어)을 공식 사이트에서 설치하세요. `setup` 마법사가 설치 여부를 감지해 없으면 공식 페이지를 안내합니다.
