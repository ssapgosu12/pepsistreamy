# PepsiStreamy — 네이티브(Rust) 버전 🦀

[루트 Python 버전](../README.md)과 **기능은 동일**하되, **런타임 의존성이 전혀 없는 단일 실행파일(.exe ~16MB)** 로 빌드되는 네이티브 구현입니다.
파이썬·numpy 설치가 필요 없어, 친구에게는 **exe 하나만** 주면 됩니다.

- 캡처: **WASAPI 루프백**([`wasapi`](https://crates.io/crates/wasapi) 크레이트) — 창 최소화/백그라운드와 무관하게 시스템 출력 믹스를 잡습니다.
- 송출: [`serenity`](https://crates.io/crates/serenity) + [`songbird`](https://crates.io/crates/songbird) — 슬래시 명령 봇, Opus 인코딩·암호화까지 처리.
- `autoconvert` 로 장치 native 포맷을 48kHz·스테레오·f32 로 변환 → songbird `RawAdapter` 에 그대로 공급.

## 받아서 바로 쓰기 (빌드 불필요)

토큰만 있으면 됩니다. [Releases](https://github.com/ssapgosu12/pepsistreamy/releases) 에서 `pepsistreamy.exe` 를 받고:

```powershell
# exe 와 같은 폴더에 .env 만들기
notepad .env        # DISCORD_TOKEN=... 한 줄
.\pepsistreamy.exe doctor    # 점검
.\pepsistreamy.exe meter     # 유튜브 틀고 막대가 움직이는지
.\pepsistreamy.exe run       # 봇 실행
```

봇 만들기/초대(최초 1회)는 [루트 README](../README.md#디스코드-봇-만들기-최초-1회)와 동일합니다.

## 소스에서 빌드

요구: **Rust(rustup)** + **Visual Studio Build Tools(C++ "x64 빌드 도구")** + **CMake**(libopus 빌드용).

```powershell
# Rust 설치(최초 1회): https://rustup.rs  (또는)  winget install Rustlang.Rustup
cd rust
cargo build --release
# 산출물: target\release\pepsistreamy.exe
Copy-Item .env.example .env   # 토큰 입력
.\target\release\pepsistreamy.exe run
```

## 명령

| 명령 | 설명 |
|------|------|
| `pepsistreamy run` | 캡처 + 봇 실행 (기본값) |
| `pepsistreamy devices` | 캡처 가능한 출력장치 목록 |
| `pepsistreamy meter [초]` | 선택 장치의 캡처 레벨 실시간 확인 (기본 10초) |
| `pepsistreamy doctor` | 토큰/기본 장치 점검 |
| 디스코드 `/join` `/leave` `/status` | 음성채널 입장/종료/상태 |

## 설정(.env)

| 키 | 필수 | 설명 |
|----|------|------|
| `DISCORD_TOKEN` | ✅ | 봇 토큰 |
| `DISCORD_GUILD_ID` | | 슬래시 명령 즉시 동기화할 서버 ID(비우면 전역, 최대 1시간) |
| `YTCAST_DEVICE` | | 캡처할 출력장치 이름 일부(비우면 기본 스피커 전체 믹스) |

`.env` 는 실행 위치(또는 상위 폴더)에서 자동 탐색됩니다.

## 구조

```
rust/
├─ Cargo.toml          # 의존성 + 릴리스 프로필(strip, lto)
└─ src/
   ├─ main.rs          # CLI: run / devices / meter / doctor
   ├─ capture.rs       # WASAPI 루프백 → 라이브 PCM 리더
   └─ bot.rs           # serenity 슬래시 명령 + songbird 송출
```

## 메모

- 특정 탭만 송출/프라이버시/문제해결은 [루트 README](../README.md)와 동일하게 적용됩니다.
- `wasapi` 크레이트에는 `new_application_loopback_client(pid, ...)`(프로세스별 루프백)도 있어, 향후 "특정 앱만" 캡처를 가상 케이블 없이 구현할 여지가 있습니다.
