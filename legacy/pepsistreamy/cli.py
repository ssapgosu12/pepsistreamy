"""pepsistreamy CLI.

  python -m pepsistreamy doctor    환경 점검(opus / PyNaCl / 토큰 / 라이브러리)
  python -m pepsistreamy devices   캡처 가능한 출력장치(스피커) 목록
  python -m pepsistreamy run       캡처 + 디스코드 봇 실행 (foreground)

설정은 환경변수 또는 프로젝트 폴더의 .env 에서 읽는다:
  DISCORD_TOKEN      (필수) 봇 토큰
  DISCORD_GUILD_ID   (선택) 슬래시 명령 즉시 동기화할 서버 ID
  YTCAST_DEVICE      (선택) 캡처할 스피커 이름(부분 일치). 없으면 기본 스피커
"""

from __future__ import annotations

import argparse
import os
import sys


def _force_utf8() -> None:
    """윈도우 기본 콘솔(cp949)에서 한국어/기호 출력이 죽지 않도록 stdout/stderr를 UTF-8로."""
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")  # type: ignore[union-attr]
        except Exception:  # noqa: BLE001
            pass


def _load_env() -> None:
    try:
        from dotenv import load_dotenv
    except ImportError:
        return
    # 현재 작업 폴더와 프로젝트 루트 양쪽의 .env 를 시도
    load_dotenv()
    here = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    load_dotenv(os.path.join(here, ".env"))


def _resolve_device(want: str | None) -> tuple[str | None, bool]:
    """YTCAST_DEVICE 부분 문자열을 (장치이름, loopback여부) 로 해석.

    먼저 출력장치(스피커=loopback)에서 찾고, 없으면 입력장치(가상 케이블 출력 등=직접녹음)에서 찾는다.
    못 찾으면 기본 스피커 루프백.
    """
    from .capture import list_input_devices, list_loopback_devices

    if not want:
        return None, True
    for name in list_loopback_devices():
        if want.lower() in name.lower():
            return name, True
    for name in list_input_devices():
        if want.lower() in name.lower():
            return name, False
    print(f"[pepsistreamy] 경고: '{want}' 와 일치하는 장치를 못 찾음. 기본 스피커 사용.", file=sys.stderr)
    return None, True


def cmd_devices(_: argparse.Namespace) -> int:
    from .capture import default_loopback_name, list_input_devices, list_loopback_devices

    default = default_loopback_name()
    print("출력장치(스피커) — 루프백 캡처 [기본 방식, 시스템 전체 믹스]:")
    for name in list_loopback_devices():
        mark = "  <- 시스템 기본" if name == default else ""
        print(f"  - {name}{mark}")

    print("\n입력장치 — 직접 녹음 [가상 케이블로 특정 앱만 분리할 때]:")
    inputs = list_input_devices()
    if inputs:
        for name in inputs:
            print(f"  - {name}")
    else:
        print("  (없음)")

    print(
        "\n특정 장치를 쓰려면 .env 에  YTCAST_DEVICE=장치이름일부  (부분일치).\n"
        "  - 시스템 전체를 그대로 보내려면 비워두세요.\n"
        "  - 특정 탭만 분리하려면 README의 '특정 탭만 송출' 절을 참고하세요."
    )
    return 0


def cmd_meter(args: argparse.Namespace) -> int:
    """선택한 장치의 캡처 레벨을 몇 초간 표시. (최소화/백그라운드 탭이 실제로 잡히는지 확인용)"""
    import math
    import struct
    import time

    _load_env()
    from .capture import FRAME_BYTES, LoopbackCapture

    device, loopback = _resolve_device(os.environ.get("YTCAST_DEVICE"))
    cap = LoopbackCapture(device_name=device, loopback=loopback)
    cap.start()
    label = device or ("기본 스피커(루프백)" if loopback else "기본 입력")
    print(f"[meter] 장치: {label}  —  {args.seconds}초간 측정. 지금 유튜브를 재생해 보세요. (Ctrl+C 중단)")

    deadline_frames = int(args.seconds * 50)  # 20ms 프레임 = 초당 50개
    peak_seen = 0.0
    try:
        for _ in range(deadline_frames):
            frame = cap.read_frame()
            n = len(frame) // 2
            if n:
                samples = struct.unpack(f"<{n}h", frame[: n * 2])
                rms = math.sqrt(sum(s * s for s in samples) / n) / 32768.0
            else:
                rms = 0.0
            peak_seen = max(peak_seen, rms)
            db = -90.0 if rms < 1e-6 else 20 * math.log10(rms)
            bars = int(max(0, (db + 60) / 60) * 40)
            sys.stdout.write(f"\r  [{'#' * bars:<40}] {db:6.1f} dBFS ")
            sys.stdout.flush()
            time.sleep(0.02)
    except KeyboardInterrupt:
        pass
    finally:
        cap.stop()

    print()
    if cap.last_error:
        print(f"[meter] 캡처 오류: {cap.last_error}", file=sys.stderr)
        return 1
    if peak_seen < 1e-4:
        print("[meter] 소리가 거의 안 잡혔습니다. 장치 선택/앱 출력 라우팅을 확인하세요.")
    else:
        print(f"[meter] 정상 — 최대 레벨 {20 * math.log10(peak_seen):.1f} dBFS. 캡처 동작합니다.")
    return 0


def cmd_doctor(_: argparse.Namespace) -> int:
    ok = True

    def check(label: str, cond: bool, hint: str = "") -> None:
        nonlocal ok
        ok = ok and cond
        mark = "OK " if cond else "X  "
        line = f"  [{mark}] {label}"
        if not cond and hint:
            line += f"\n         -> {hint}"
        print(line)

    print("pepsistreamy 환경 점검")

    try:
        import numpy  # noqa: F401
        check("numpy", True)
    except Exception as e:  # noqa: BLE001
        check("numpy", False, f"pip install numpy ({e})")

    try:
        import soundcard  # noqa: F401
        check("soundcard", True)
    except Exception as e:  # noqa: BLE001
        check("soundcard", False, f"pip install soundcard ({e})")

    try:
        import discord  # noqa: F401
        check("discord.py", True)
    except Exception as e:  # noqa: BLE001
        check("discord.py", False, f"pip install \"discord.py[voice]\" ({e})")
        print("\n핵심 라이브러리가 없어 추가 점검을 건너뜁니다.")
        return 1

    try:
        import nacl  # noqa: F401
        check("PyNaCl (음성 암호화)", True)
    except Exception:  # noqa: BLE001
        check("PyNaCl (음성 암호화)", False, "pip install \"discord.py[voice]\"")

    try:
        import discord

        loaded = discord.opus.is_loaded()
        if not loaded:
            try:
                discord.opus._load_default()  # type: ignore[attr-defined]
                loaded = discord.opus.is_loaded()
            except Exception:  # noqa: BLE001
                loaded = False
        check("libopus (음성 인코딩)", loaded,
              "윈도우는 보통 discord.py에 동봉돼 자동 로드됩니다. 안 되면 libopus-0.x64.dll 필요.")
    except Exception as e:  # noqa: BLE001
        check("libopus (음성 인코딩)", False, str(e))

    try:
        import shutil
        check("ffmpeg (PATH)", shutil.which("ffmpeg") is not None,
              "있으면 좋지만 라이브 PCM 송출에는 필수는 아님.")
    except Exception:  # noqa: BLE001
        pass

    _load_env()
    check("DISCORD_TOKEN 설정됨", bool(os.environ.get("DISCORD_TOKEN")),
          ".env 에 DISCORD_TOKEN=... 추가 (Discord 개발자 포털 > Bot > Reset Token)")

    print("\n결과:", "정상 — `python -m pepsistreamy run` 가능" if ok else "위 X 항목을 해결하세요.")
    return 0 if ok else 1


def cmd_run(_: argparse.Namespace) -> int:
    _load_env()
    token = os.environ.get("DISCORD_TOKEN")
    if not token:
        print("DISCORD_TOKEN 이 없습니다. .env 에 설정하세요. (자세한 건 README)", file=sys.stderr)
        return 1

    guild_id_raw = os.environ.get("DISCORD_GUILD_ID")
    guild_id = int(guild_id_raw) if guild_id_raw and guild_id_raw.isdigit() else None

    from .bot import build_bot
    from .capture import LoopbackCapture

    device, loopback = _resolve_device(os.environ.get("YTCAST_DEVICE"))
    capture = LoopbackCapture(device_name=device, loopback=loopback)
    src = device or ("기본 스피커(시스템 전체 믹스, 루프백)" if loopback else "기본 입력")
    print(f"[pepsistreamy] 캡처 소스: {src}")
    bot = build_bot(capture, guild_id=guild_id)

    print("[pepsistreamy] 봇 시작 중...  (Ctrl+C 로 종료)")
    try:
        bot.run(token, log_handler=None)
    except KeyboardInterrupt:
        pass
    finally:
        capture.stop()
    return 0


def main(argv: list[str] | None = None) -> int:
    _force_utf8()
    parser = argparse.ArgumentParser(prog="pepsistreamy", description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("doctor", help="환경 점검").set_defaults(func=cmd_doctor)
    sub.add_parser("devices", help="장치 목록").set_defaults(func=cmd_devices)
    p_meter = sub.add_parser("meter", help="캡처 레벨 확인")
    p_meter.add_argument("--seconds", type=float, default=10.0, help="측정 시간(초)")
    p_meter.set_defaults(func=cmd_meter)
    sub.add_parser("run", help="캡처+봇 실행").set_defaults(func=cmd_run)

    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
