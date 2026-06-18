"""디스코드 봇 — 음성 채널에 들어가 루프백 캡처를 실시간 송출한다.

슬래시 명령만 사용하므로 특권 인텐트(Message Content / Members / Presence)가 필요 없다.
voice_states 인텐트는 비특권이라 그대로 켠다(명령 친 사람이 어느 음성 채널에 있는지 읽기 위함).

명령:
  /join    명령 친 사람이 있는 음성 채널로 들어가 방송 시작
  /leave   방송 종료 후 음성 채널 나가기
  /status  캡처/연결 상태 보기
"""

from __future__ import annotations

import discord
from discord import app_commands

from .capture import LoopbackCapture
from .source import LoopbackSource


def build_bot(capture: LoopbackCapture, guild_id: int | None = None) -> discord.Client:
    intents = discord.Intents.none()
    intents.guilds = True
    intents.voice_states = True
    client = discord.Client(intents=intents)
    tree = app_commands.CommandTree(client)
    client.tree = tree  # type: ignore[attr-defined]  # 접근/동기화 편의

    @client.event
    async def setup_hook() -> None:  # noqa: D401
        # 길드 지정 시 즉시 동기화(전역 동기화는 최대 1시간 걸림)
        if guild_id:
            g = discord.Object(id=guild_id)
            tree.copy_global_to(guild=g)
            await tree.sync(guild=g)
        else:
            await tree.sync()

    @client.event
    async def on_ready() -> None:  # noqa: D401
        print(f"[pepsistreamy] 로그인됨: {client.user}  (서버 {len(client.guilds)}개)")
        print("[pepsistreamy] 디스코드에서 음성 채널 입장 후 /join 입력하세요.")

    @tree.command(description="내가 있는 음성 채널로 들어가 유튜브 소리 방송을 시작합니다")
    async def join(interaction: discord.Interaction) -> None:
        member = interaction.user
        voice_state = getattr(member, "voice", None)
        if voice_state is None or voice_state.channel is None:
            await interaction.response.send_message(
                "먼저 음성 채널에 들어간 다음 다시 `/join` 하세요.", ephemeral=True
            )
            return

        channel = voice_state.channel
        await interaction.response.defer(ephemeral=True)

        vc = interaction.guild.voice_client
        if vc is None:
            vc = await channel.connect(self_deaf=True)
        elif vc.channel.id != channel.id:
            await vc.move_to(channel)

        if not capture.running:
            capture.start()

        if vc.is_playing():
            vc.stop()
        vc.play(LoopbackSource(capture))

        await interaction.followup.send(
            f"▶️ **{channel.name}** 에서 방송 시작. 브라우저에서 유튜브를 재생하세요.",
            ephemeral=True,
        )

    @tree.command(description="방송을 멈추고 음성 채널에서 나갑니다")
    async def leave(interaction: discord.Interaction) -> None:
        vc = interaction.guild.voice_client
        if vc is None:
            await interaction.response.send_message("연결돼 있지 않습니다.", ephemeral=True)
            return
        if vc.is_playing():
            vc.stop()
        await vc.disconnect(force=True)
        await interaction.response.send_message("⏹️ 방송 종료, 채널에서 나갔습니다.", ephemeral=True)

    @tree.command(description="캡처/연결 상태를 봅니다")
    async def status(interaction: discord.Interaction) -> None:
        vc = interaction.guild.voice_client
        lines = [
            f"캡처: {'동작중' if capture.running else '중지'}"
            + (f" (오류: {capture.last_error})" if capture.last_error else ""),
            f"채널: {vc.channel.name if vc and vc.channel else '미연결'}",
            f"재생중: {bool(vc and vc.is_playing())}",
            f"프레임: 캡처 {capture.frames_captured} / 드롭 {capture.frames_dropped} / 버퍼 {capture.q.qsize()}",
            f"장치: {capture.device_name or '기본 스피커'}",
        ]
        await interaction.response.send_message("```\n" + "\n".join(lines) + "\n```", ephemeral=True)

    return client
