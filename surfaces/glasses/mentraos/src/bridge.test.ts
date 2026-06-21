import { test } from "node:test";
import assert from "node:assert/strict";
import {
  parseEvent,
  paginate,
  renderEvent,
  DisplayCoalescer,
  DEFAULT_DISPLAY,
  type CoachEvent,
  type GlassesView,
} from "./bridge";

test("parseEvent: valid frame parses, garbage is null", () => {
  const ok = parseEvent('{"type":"prompt","kind":"cue","text":"hi","priority":3,"ttl_ms":12000,"t":1,"prompt_id":"p1"}');
  assert.equal(ok?.type, "prompt");
  assert.equal(parseEvent("not json"), null);
  assert.equal(parseEvent('{"no":"type"}'), null);
});

test("paginate: short text stays one line", () => {
  assert.equal(paginate("hello world", 32, 4), "hello world");
});

test("paginate: wraps at maxChars", () => {
  const out = paginate("alpha bravo charlie delta echo", 12, 4);
  const lines = out.split("\n");
  assert.ok(lines.every((l) => l.length <= 12), `line too long: ${JSON.stringify(lines)}`);
  assert.ok(lines.length >= 2);
});

test("paginate: caps lines and ellipsizes on overflow", () => {
  const long = Array.from({ length: 60 }, (_, i) => `w${i}`).join(" ");
  const out = paginate(long, 10, 3);
  const lines = out.split("\n");
  assert.equal(lines.length, 3);
  assert.ok(out.endsWith("…"), `expected ellipsis, got ${JSON.stringify(out)}`);
  assert.ok(lines.every((l) => l.length <= 10));
});

test("paginate: empty input", () => {
  assert.equal(paginate("   ", 32, 4), "");
});

test("renderEvent: prompt -> reference card with KIND title and ttl duration", () => {
  const ev: CoachEvent = { type: "prompt", kind: "objection", text: "Anchor to ROI", priority: 4, ttl_ms: 9000, t: 5, prompt_id: "p1" };
  const v = renderEvent(ev, DEFAULT_DISPLAY) as GlassesView & { kind: "reference" };
  assert.equal(v.kind, "reference");
  assert.equal(v.title, "OBJECTION");
  assert.equal(v.durationMs, 9000);
  assert.ok(v.body.includes("Anchor"));
});

test("renderEvent: transcript.final -> text, them vs you", () => {
  const them = renderEvent({ type: "transcript.final", speaker: "them", text: "hello there", t: 1 }, DEFAULT_DISPLAY);
  assert.equal(them?.kind, "text");
  assert.ok((them as any).body.startsWith("them:"));
  const me = renderEvent({ type: "transcript.final", speaker: "me", text: "hi", t: 1 }, DEFAULT_DISPLAY);
  assert.ok((me as any).body.startsWith("you:"));
});

test("renderEvent: transcript suppressed when showTranscript=false", () => {
  const v = renderEvent({ type: "transcript.final", speaker: "them", text: "x", t: 1 }, { ...DEFAULT_DISPLAY, showTranscript: false });
  assert.equal(v, null);
});

test("renderEvent: partials and state render nothing", () => {
  assert.equal(renderEvent({ type: "transcript.partial", speaker: "them", text: "x", t: 1 }, DEFAULT_DISPLAY), null);
  assert.equal(renderEvent({ type: "state", capturing: true, model: "m", t: 1 }, DEFAULT_DISPLAY), null);
});

test("DisplayCoalescer: respects min interval", () => {
  const c = new DisplayCoalescer(300);
  c.push({ kind: "text", body: "a" }, false);
  assert.ok(c.flush(0), "first flush should emit");
  c.push({ kind: "text", body: "b" }, false);
  assert.equal(c.flush(100), null, "within interval -> null");
  assert.ok(c.flush(300), "after interval -> emit");
});

test("DisplayCoalescer: prompt overrides pending transcript, not vice versa", () => {
  const c = new DisplayCoalescer(300);
  c.flush(0); // prime lastEmit=0
  c.push({ kind: "text", body: "transcript" }, false);
  c.push({ kind: "reference", title: "CUE", body: "do x" }, true);
  c.push({ kind: "text", body: "newer transcript" }, false); // must NOT override the prompt
  const v = c.flush(300) as any;
  assert.equal(v.kind, "reference");
  assert.equal(v.title, "CUE");
});

test("DisplayCoalescer: nothing pending -> null", () => {
  const c = new DisplayCoalescer(300);
  assert.equal(c.flush(1000), null);
});
