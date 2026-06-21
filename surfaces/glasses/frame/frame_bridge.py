"""Souffleur -> Brilliant Labs Frame surface (PC-direct, no cloud).

Connects to the Souffleur core's Coach Protocol WebSocket and pushes prompts +
transcript to the Frame lens over BLE using the real `frame-sdk` API. This is the
fully-open, no-phone, no-cloud reference path (vs the MentraOS cloud surface).

HARDWARE REQUIRED to run: Brilliant Labs Frame glasses + a BLE adapter on this
machine (frame-sdk uses Bleak). `async with Frame()` performs BLE discovery and
will block/fail without the glasses. The bridge logic (bridge.py) is verified by
unit tests; the BLE display path is verified only on real hardware.

Run (with Frame paired):  SOUFFLEUR_WS=ws://127.0.0.1:8123 python frame_bridge.py
"""
from __future__ import annotations

import asyncio
import os

import websockets
from frame_sdk import Frame
from frame_sdk.display import Alignment

from bridge import parse_event, render_event

SOUFFLEUR_WS = os.environ.get("SOUFFLEUR_WS", "ws://127.0.0.1:8123")
MAX_CHARS = int(os.environ.get("FRAME_CHARS_PER_LINE", "40"))
MAX_LINES = int(os.environ.get("FRAME_LINES", "5"))


async def pump(frame: Frame) -> None:
    """Read Coach Protocol frames and render them to the lens until the socket closes."""
    async with websockets.connect(SOUFFLEUR_WS) as ws:
        await frame.display.show_text("Souffleur connected", align=Alignment.MIDDLE_CENTER)
        async for raw in ws:
            text = raw if isinstance(raw, str) else raw.decode("utf-8", "ignore")
            ev = parse_event(text)
            if ev is None:
                continue
            view = render_event(ev, max_chars=MAX_CHARS, max_lines=MAX_LINES)
            if view is None:
                continue
            await frame.display.clear()
            if view.scroll:
                await frame.display.scroll_text(view.text)
            else:
                await frame.display.show_text(view.text, align=Alignment.TOP_LEFT)


async def main() -> None:
    # `async with Frame()` connects over BLE — requires the real glasses.
    async with Frame() as frame:
        while True:
            try:
                await pump(frame)
            except websockets.ConnectionClosed:
                await asyncio.sleep(1.5)  # core went away; retry


if __name__ == "__main__":
    asyncio.run(main())
