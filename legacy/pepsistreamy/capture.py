"""WASAPI 루프백 캡처.

지금 스피커로 나가는 소리(=브라우저에서 재생 중인 유튜브 포함)를 그대로 잡아서
20ms짜리 48kHz / 스테레오 / s16le PCM 프레임 큐로 만든다. 추가 드라이버(가상 케이블) 없이
soundcard 라이브러리의 WASAPI loopback 기능만으로 동작한다.

Discord 음성 표준이 48kHz·2ch·20ms(=프레임당 3840바이트)라 처음부터 그 포맷으로 맞춰 캡처한다.
"""

from __future__ import annotations

import queue
import threading

import numpy as np
import soundcard as sc

SAMPLE_RATE = 48_000                                  # Discord 음성 표준
CHANNELS = 2
FRAME_MS = 20
FRAME_SAMPLES = SAMPLE_RATE * FRAME_MS // 1000        # 960
FRAME_BYTES = FRAME_SAMPLES * CHANNELS * 2            # 3840 (s16le)
SILENCE = b"\x00" * FRAME_BYTES


def list_loopback_devices() -> list[str]:
    """루프백으로 캡처 가능한 출력장치 이름 목록 (= 스피커들)."""
    return [spk.name for spk in sc.all_speakers()]


def list_input_devices() -> list[str]:
    """직접 녹음 가능한 입력장치 이름 목록 (가상 케이블의 'CABLE Output' 등)."""
    return [m.name for m in sc.all_microphones(include_loopback=False)]


def default_loopback_name() -> str:
    return sc.default_speaker().name


class LoopbackCapture:
    """별도 스레드에서 루프백 오디오를 계속 읽어 큐에 쌓는다.

    큐는 최신 우선(bounded). 가득 차면 가장 오래된 프레임을 버려 지연이 무한정 늘지 않게 한다.
    """

    def __init__(
        self,
        device_name: str | None = None,
        loopback: bool = True,
        max_queue_frames: int = 50,
    ) -> None:
        # loopback=True  : 출력장치(스피커) 믹스를 루프백으로 캡처 (기본)
        # loopback=False : 입력장치(가상 케이블 출력 등)를 직접 녹음
        self.device_name = device_name
        self.loopback = loopback
        self.q: queue.Queue[bytes] = queue.Queue(maxsize=max_queue_frames)
        self._thread: threading.Thread | None = None
        self._stop = threading.Event()
        self.frames_captured = 0
        self.frames_dropped = 0
        self.last_error: str | None = None

    @property
    def running(self) -> bool:
        return self._thread is not None and self._thread.is_alive()

    def _resolve_mic(self):
        # loopback=True  → 출력장치를 입력처럼 잡음 (WASAPI loopback)
        # loopback=False → 진짜 입력장치를 그대로 녹음
        if self.loopback:
            name = self.device_name or sc.default_speaker().name
            return sc.get_microphone(name, include_loopback=True)
        name = self.device_name or sc.default_microphone().name
        return sc.get_microphone(name, include_loopback=False)

    def _run(self) -> None:
        try:
            mic = self._resolve_mic()
            with mic.recorder(
                samplerate=SAMPLE_RATE, channels=CHANNELS, blocksize=FRAME_SAMPLES
            ) as rec:
                while not self._stop.is_set():
                    data = rec.record(numframes=FRAME_SAMPLES)  # (960, 2) float32
                    pcm = np.clip(data, -1.0, 1.0)
                    pcm = (pcm * 32767.0).astype("<i2").tobytes()
                    if len(pcm) < FRAME_BYTES:                  # 모자라면 무음 패딩
                        pcm = pcm + SILENCE[len(pcm):]
                    elif len(pcm) > FRAME_BYTES:
                        pcm = pcm[:FRAME_BYTES]
                    self._push(pcm)
        except Exception as exc:  # noqa: BLE001 — 스레드 안에서 죽지 않게 잡아 보고만
            self.last_error = f"{type(exc).__name__}: {exc}"

    def _push(self, pcm: bytes) -> None:
        try:
            self.q.put_nowait(pcm)
            self.frames_captured += 1
        except queue.Full:
            # 가장 오래된 프레임 버리고 새 프레임 넣기 → 항상 '지금'에 가까운 소리
            try:
                self.q.get_nowait()
                self.frames_dropped += 1
            except queue.Empty:
                pass
            try:
                self.q.put_nowait(pcm)
                self.frames_captured += 1
            except queue.Full:
                pass

    def read_frame(self) -> bytes:
        """20ms PCM 한 프레임. 버퍼가 비었으면 무음(언더런 보호)."""
        try:
            return self.q.get_nowait()
        except queue.Empty:
            return SILENCE

    def start(self) -> None:
        if self.running:
            return
        self._stop.clear()
        self.last_error = None
        self._thread = threading.Thread(target=self._run, name="pepsistreamy-loopback", daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        if self._thread:
            self._thread.join(timeout=2)
        self._thread = None
