"""캡처 큐 → Discord 음성으로 잇는 AudioSource.

discord.py의 음성 플레이어는 20ms마다 read()를 호출해 정확히 3840바이트(s16le, 48k, 2ch)를
기대한다. 라이브 소스라 read()는 절대 b''를 돌려주지 않는다(그러면 재생이 멈춘다). 버퍼가 비면
무음을 돌려줘 연결만 유지한다.
"""

from __future__ import annotations

import discord

from .capture import LoopbackCapture


class LoopbackSource(discord.AudioSource):
    def __init__(self, capture: LoopbackCapture) -> None:
        self.capture = capture

    def read(self) -> bytes:
        return self.capture.read_frame()  # 항상 3840바이트

    def is_opus(self) -> bool:
        return False  # PCM을 주면 discord.py가 Opus로 인코딩한다

    def cleanup(self) -> None:
        pass
