# PepsiStreamy v2 — 네이티브(Rust) 🦀🥤

내 PC 소리(브라우저의 유튜브 등)를 디스코드 음성 채널로 실시간 방송하는 **단일 실행파일(.exe, ~16MB, 런타임 의존성 0)**. 파이썬 설치가 필요 없습니다.

> 이것이 현재 버전(v2)입니다. 이전 Python 구현(v1)은 [`legacy/`](legacy/) 에 보존되어 있습니다.

- 🖥️ **설정 TUI** — 인자 없이 실행하면 방향키 TUI가 열림(토큰·소스·DSP·모니터).
- 🔐 **토큰 암호화** — 봇 토큰은 Windows DPAPI로 암호화되어 `setting.ini` 에 저장. **파일을 공유해도 다른 계정/PC에선 복호화 불가**(DSP·소스 설정만 공유됨).
- 🎯 **특정 앱만 캡처** — 프로세스 루프백. 가상 케이블 불필요. 프로세스 목록은 **지금 소리 나는 앱만**(오디오 세션 기준) 추려 보여줌(크롬도 한 줄, `a`키로 전체).
- 🎚️ **내장 DSP** — HP/LP + 리버브(노브 6개) + 커스텀 프로필 저장.
- 🎧 **로컬 모니터** — 송출자도 청취자와 같은 *필터된* 소리를 들을 수 있음.
- 📊 **실행 화면** — 봇 실행 시 라이브 볼륨바·연결/청취자 상태 표시. **Esc → 설정으로 복귀**(거기서 DSP/장치 바꿔 다시 실행), **Ctrl+C → 종료**.
- 🔁 **무중단 변경** — 디스코드 `/reload` 로 채널을 나가지 않고 소스/DSP/모니터 재적용.
- 🚪 **자동 퇴장** — 채널에 사람이 아무도 없으면 봇이 자동으로 나감.
- 송출: serenity + songbird(Opus). 라이브 PCM 리더는 논블로킹(끊김 완화).

## 빠른 시작

[Releases](https://github.com/ssapgosu12/pepsistreamy/releases) 에서 `pepsistreamy.exe` 다운로드 후:

```powershell
.\pepsistreamy.exe          # ← 설정 TUI 가 열립니다 (인자 없음)
```

TUI 메뉴에서:
1. **디스코드 토큰 설정** — 봇 토큰 붙여넣기(즉시 암호화 저장). 토큰에서 초대링크를 자동 생성해 브라우저로 열어줍니다.
2. **캡처 소스** — 기본 스피커 / 특정 장치 / 특정 프로세스 / 레거시(VB-CABLE)
3. **DSP 필터** — HP/LP/리버브 조정, 프로필 저장
4. **로컬 모니터** — 송출자도 필터본 듣기
5/6. **저장** 또는 **저장하고 봇 실행**

봇을 내 서버에 초대(최초 1회)는 토큰 설정 시 자동으로 초대 페이지가 열립니다. 안 열리면 개발자 포털 > OAuth2 > URL Generator(스코프 `bot`+`applications.commands`, 권한 View Channels/Connect/Speak).

봇 실행 후 디스코드에서 음성 채널 입장 → `/join`.

## 토큰 암호화 & setting.ini 공유

`setting.ini` 는 이렇게 저장됩니다:
- `[auth] token` = **DPAPI 암호문**(현재 Windows 계정/PC 전용). 다른 사람이 받아도 **복호화 안 됨**.
- `[source]`, `[dsp]`, `[monitor]`, `[profiles]` = **평문**(자유롭게 공유 가능).

즉 친구에게 `setting.ini` 를 주면 **DSP/소스/프로필 세팅만 공유**되고, **봇 토큰은 안전**합니다. (각자 자기 토큰을 TUI에서 넣으면 됩니다.)

## 특정 앱만 캡처 (프로세스 루프백)

TUI > 캡처 소스 > "특정 프로세스" 에서 목록을 보고 고르거나, `setting.ini` 의 `[source] kind=process, value=chrome`(이름 또는 PID).

- 대상 프로세스 + **자식 프로세스**까지 캡처(가상 케이블 불필요).
- **크롬은 모든 탭을 한 오디오 프로세스에서 섞으므로** 크롬을 지정하면 모든 탭이 잡힙니다 — 단일 탭만 분리하려면 그 탭을 **전용 브라우저/프로필**에 띄우세요.

## 내장 DSP + 프로필

TUI > DSP 필터. 노브 6개(각 0~100):

| 노브 | 의미 | 기본 |
|------|------|------|
| HighFreq | 하이패스 컷오프 | 10 |
| HighRes | 하이패스 Q(레조넌스) | 40 |
| LowFreq | 로우패스 컷오프 | 53 |
| LowRes | 로우패스 Q | 10 |
| RevRoomSize | 리버브 룸 크기 | 60 |
| RevMix | 리버브 섞임(wet) | 25 |

`←→`(또는 `+`/`-`)로 값 조정, `t` 토글, `s` 프로필 저장, `l` 프로필 불러오기. 목소리 대역(≈300–3400Hz) 위쪽을 깎고 리버브를 더해 배경음이 대화/게임에 덜 묻힙니다.

## 🎧 송출자도 "필터된" 소리 듣기 (로컬 모니터)

기본적으로 송출자 스피커에는 **원본**이 나오고 봇에만 필터본이 갑니다. 송출자도 청취자와 같은 필터본을 들으려면:

1. **캡처 소스 = 특정 프로세스**(그 앱)로 설정
2. **로컬 모니터 = 켜짐**(TUI > 로컬 모니터) — pepsistreamy가 *필터된* 소리를 스피커로 재생
3. **Windows 볼륨 믹서에서 그 앱을 뮤트** (⚠️ 크롬 *탭* 뮤트 말고 **볼륨 믹서의 앱 뮤트**)

왜 되나: 프로세스 루프백은 **pre-volume** 캡처라 볼륨 믹서에서 앱을 뮤트해도 **캡처는 살아있습니다**. 그래서 원본은 스피커에서 사라지고(앱 뮤트), 모니터가 필터본만 재생 → **송출자·봇 모두 필터본**. 옛 "VB-CABLE+크롬 확장" 방식과 같은 효과를 모든 앱에서.

> 일부 앱/장치가 post-volume라 뮤트 시 캡처까지 죽으면: **레거시 모드(VB-CABLE)** 로 그 앱 출력을 CABLE 로 보내고(=스피커에서 안 들림) 모니터로 필터본을 재생하세요. TUI > 캡처 소스 > 레거시에서 안내합니다.

## 소리가 안 나와요 / 볼륨바 -90 고정

먼저 **`pepsistreamy meter`** 를 실행하세요(setting.ini 와 똑같은 설정으로 캡처만 테스트). 막대가 움직이면 캡처는 정상이고 봇도 됩니다. -90 이면 소스/라우팅 문제입니다:

- **레거시(VB-CABLE)**: 그 앱의 출력을 **TUI에서 고른 바로 그 케이블**(예: `CABLE Input`)로 보냈는지 확인. 케이블이 여러 개면(`CABLE Input` vs `CABLE In 16ch`) **앱이 보낸 케이블과 캡처하는 케이블이 같아야** 합니다.
- **특정 프로세스**: 그 앱이 실제로 소리를 내는지 확인(소리 나면 캡처 소스 목록 🔊 에 떠야 함). 크롬은 단일 탭이면 그 탭이 재생 중이어야 합니다.
- 자세한 진단: `YTCAST_DEBUG=1 pepsistreamy meter` 로 매칭된 장치/샘플값을 볼 수 있습니다.

## 명령

| 명령 | 설명 |
|------|------|
| `pepsistreamy` (인자 없음) | 설정 TUI |
| `pepsistreamy run` | 캡처 + 봇 실행 (setting.ini 사용) |
| `pepsistreamy devices` | 출력장치 목록 |
| `pepsistreamy processes` | 실행 중 프로세스 목록 |
| `pepsistreamy meter [초]` | 캡처 레벨 확인 |
| `pepsistreamy doctor` | 점검 |
| 디스코드 `/join` `/leave` `/status` | 입장/종료/상태 |
| 디스코드 `/reload` | setting.ini 의 소스/DSP/모니터를 채널 유지한 채 재적용 |

**실행 화면**: "저장하고 봇 실행" 하면 라이브 볼륨바·상태 화면이 뜹니다. `Esc`/`q` 로 설정 메뉴로 돌아가 DSP·장치를 바꾸고 다시 실행할 수 있고, `Ctrl+C` 로 완전히 종료합니다. 채널에 사람이 다 나가면 봇은 자동 퇴장합니다.

환경변수(`DISCORD_TOKEN`, `YTCAST_PROCESS`, `YTCAST_DSP` 등)로 setting.ini 를 덮어쓸 수 있습니다.

## 소스에서 빌드

요구: **Rust(rustup)** + **Visual Studio Build Tools(C++ x64)** + **CMake**(libopus).

```powershell
cd rust
cargo build --release   # target\release\pepsistreamy.exe
cargo test              # b64 / DPAPI 암호화 / setting.ini 왕복 / 토큰 디코드
```

## 구조

```
src/
├─ main.rs       CLI 디스패치 (인자 없음 → TUI)
├─ tui.rs        ratatui 설정 TUI (메뉴/토큰/소스/DSP/모니터)
├─ settings.rs   setting.ini + DPAPI 토큰 암호화 + 프로필
├─ capture.rs    WASAPI 루프백(장치/프로세스) → 논블로킹 라이브 PCM
├─ dsp.rs        biquad HP/LP + Freeverb + 0~100 노브 매핑
├─ monitor.rs    필터본을 출력장치로 재생(로컬 모니터)
├─ process.rs    프로세스 열거/이름→PID
├─ setup.rs      개발자포털/초대URL/토큰 디코드 보조
├─ b64.rs        base64
└─ bot.rs        serenity 슬래시 명령 + songbird 송출
```

## VB-CABLE

레거시 모드에서만 안내합니다. [VB-CABLE](https://vb-audio.com/Cable/)(VB-Audio 도네이션웨어)을 공식 사이트에서 설치하세요. TUI가 설치 여부를 감지해 없으면 공식 페이지를 안내합니다(바이너리 번들 없이 링크만).
