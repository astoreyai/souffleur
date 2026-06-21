"""Real unit tests for the Frame bridge (hardware-independent)."""
from bridge import FrameView, paginate, parse_event, render_event


def test_parse_event_valid_and_garbage():
    ok = parse_event('{"type":"prompt","kind":"cue","text":"hi","priority":3,"ttl_ms":12000,"t":1,"prompt_id":"p1"}')
    assert ok is not None and ok["type"] == "prompt"
    assert parse_event("not json") is None
    assert parse_event('{"no":"type"}') is None


def test_paginate_short_stays_one_line():
    assert paginate("hello world", 40, 5) == "hello world"


def test_paginate_wraps_at_max_chars():
    out = paginate("alpha bravo charlie delta echo", 12, 4)
    lines = out.split("\n")
    assert all(len(l) <= 12 for l in lines), lines
    assert len(lines) >= 2


def test_paginate_caps_and_ellipsizes():
    long = " ".join(f"w{i}" for i in range(60))
    out = paginate(long, 10, 3)
    lines = out.split("\n")
    assert len(lines) == 3
    assert out.endswith("…")
    assert all(len(l) <= 10 for l in lines)


def test_paginate_empty():
    assert paginate("   ", 40, 5) == ""


def test_render_prompt_to_titled_view_with_duration():
    v = render_event({"type": "prompt", "kind": "objection", "text": "Anchor to ROI", "priority": 4, "ttl_ms": 9000, "t": 5})
    assert isinstance(v, FrameView)
    assert v.text.startswith("OBJECTION")
    assert "Anchor" in v.text
    assert v.duration_s == 9.0
    assert v.scroll is False


def test_render_transcript_them_vs_you():
    them = render_event({"type": "transcript.final", "speaker": "them", "text": "hello there", "t": 1})
    assert them is not None and them.text.startswith("them:")
    me = render_event({"type": "transcript.final", "speaker": "me", "text": "hi", "t": 1})
    assert me is not None and me.text.startswith("you:")


def test_render_transcript_suppressed():
    assert render_event({"type": "transcript.final", "speaker": "them", "text": "x", "t": 1}, show_transcript=False) is None


def test_render_partials_and_state_are_none():
    assert render_event({"type": "transcript.partial", "speaker": "them", "text": "x", "t": 1}) is None
    assert render_event({"type": "state", "capturing": True, "model": "m", "t": 1}) is None


def test_render_long_transcript_scrolls():
    long_text = "word " * 80
    v = render_event({"type": "transcript.final", "speaker": "them", "text": long_text, "t": 1})
    assert v is not None and v.scroll is True
