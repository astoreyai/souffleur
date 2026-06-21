"""Hardware-independent bridge: Coach Protocol event -> what to show on a
Brilliant Labs Frame lens. Pure functions, unit-testable without the glasses.

Frame is 640x400 micro-OLED (higher-res than the narrow green HUDs), so defaults
are more generous, but chars-per-line is still hardware-dependent (the SDK's own
wrap_text/show_text(max_width=640) does pixel wrapping on-device); these are the
pre-wrap caps used to keep the cue glanceable. Measure on the real Frame.
"""
from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Optional


@dataclass
class FrameView:
    text: str
    scroll: bool
    duration_s: Optional[float]


def parse_event(raw: str) -> Optional[dict]:
    """Parse one NDJSON Coach Protocol frame; None on garbage."""
    try:
        o = json.loads(raw)
    except (ValueError, TypeError):
        return None
    return o if isinstance(o, dict) and isinstance(o.get("type"), str) else None


def paginate(text: str, max_chars: int, max_lines: int) -> str:
    """Word-wrap to max_chars and cap at max_lines, ellipsizing on overflow."""
    words = text.split()
    lines: list[str] = []
    cur = ""
    overflow = False
    for w in words:
        nxt = w if cur == "" else cur + " " + w
        if len(nxt) <= max_chars:
            cur = nxt
        else:
            if cur:
                lines.append(cur)
            cur = w if len(w) <= max_chars else w[:max_chars]
            if len(lines) >= max_lines:
                overflow = True
                break
    if not overflow and cur and len(lines) < max_lines:
        lines.append(cur)
    if len(lines) > max_lines:
        lines = lines[:max_lines]
    if overflow:
        last = lines[max_lines - 1] if len(lines) >= max_lines else cur
        if len(last) > max_chars - 1:
            last = last[: max_chars - 1]
        if len(lines) >= max_lines:
            lines[max_lines - 1] = last + "…"
        else:
            lines.append(last + "…")
    return "\n".join(lines)


def render_event(
    ev: dict,
    max_chars: int = 40,
    max_lines: int = 5,
    show_transcript: bool = True,
) -> Optional[FrameView]:
    """Map a Coach Protocol event to a Frame view, or None if nothing to show."""
    t = ev.get("type")
    if t == "prompt":
        title = str(ev.get("kind", "note")).upper()
        body = paginate(str(ev.get("text", "")), max_chars, max_lines - 1)
        ttl = ev.get("ttl_ms")
        return FrameView(
            text=f"{title}\n{body}",
            scroll=False,
            duration_s=(float(ttl) / 1000.0) if isinstance(ttl, (int, float)) else None,
        )
    if t == "transcript.final":
        if not show_transcript:
            return None
        who = "you" if str(ev.get("speaker", "")).startswith("me") else "them"
        body = f"{who}: {ev.get('text', '')}"
        return FrameView(text=body, scroll=len(body) > max_chars * max_lines, duration_s=None)
    return None
