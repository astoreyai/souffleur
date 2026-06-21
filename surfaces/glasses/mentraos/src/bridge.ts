/**
 * Hardware-independent bridge: Coach Protocol events -> what to show on the lens.
 *
 * This is the part that can be unit-tested without glasses. The MentraOS app
 * (index.ts) wires it to the real SDK (`session.layouts.*`). The display geometry
 * (chars-per-line, lines) is CONFIG because it is undocumented on every glasses
 * vendor and must be measured on real hardware (see docs/research/04 §4).
 */

export type CoachEvent =
  | { type: "transcript.partial"; speaker: string; text: string; t: number }
  | { type: "transcript.final"; speaker: string; text: string; t: number; stt_latency_ms?: number }
  | { type: "prompt"; prompt_id: string; kind: string; text: string; priority: number; ttl_ms: number; t: number }
  | { type: "state"; capturing: boolean; model: string; t: number }
  | { type: string; [k: string]: unknown };

/** A render instruction the app maps to a MentraOS layout call. */
export type GlassesView =
  | { kind: "reference"; title: string; body: string; durationMs?: number }
  | { kind: "text"; body: string; durationMs?: number };

export interface DisplayConfig {
  /** Characters per line on the lens. HARDWARE-MEASURED — placeholder default. */
  maxCharsPerLine: number;
  /** Lines visible on the lens. HARDWARE-MEASURED — placeholder default. */
  maxLines: number;
  /** Show the other party's transcript when no prompt is active. */
  showTranscript: boolean;
}

/** Conservative defaults for a narrow green HUD (e.g. Even Realities G1 / Vuzix Z100).
 *  These are NOT vendor-confirmed; measure on the target glasses and override. */
export const DEFAULT_DISPLAY: DisplayConfig = {
  maxCharsPerLine: 32,
  maxLines: 4,
  showTranscript: true,
};

/** Parse one NDJSON Coach Protocol frame; returns null on garbage. */
export function parseEvent(raw: string): CoachEvent | null {
  try {
    const o = JSON.parse(raw);
    if (o && typeof o.type === "string") return o as CoachEvent;
    return null;
  } catch {
    return null;
  }
}

/** Word-wrap to maxChars and cap at maxLines, ellipsizing if it overflows. */
export function paginate(text: string, maxChars: number, maxLines: number): string {
  const words = text.trim().split(/\s+/).filter(Boolean);
  const lines: string[] = [];
  let cur = "";
  let overflow = false;
  for (let i = 0; i < words.length; i++) {
    const w = words[i];
    const next = cur === "" ? w : cur + " " + w;
    if (next.length <= maxChars) {
      cur = next;
    } else {
      if (cur !== "") lines.push(cur);
      cur = w.length <= maxChars ? w : w.slice(0, maxChars);
      if (lines.length >= maxLines) {
        overflow = true;
        break;
      }
    }
  }
  if (!overflow && cur !== "" && lines.length < maxLines) lines.push(cur);
  if (lines.length > maxLines) lines.length = maxLines;
  if (overflow) {
    let last = lines[maxLines - 1] ?? cur;
    if (last.length > maxChars - 1) last = last.slice(0, maxChars - 1);
    lines[maxLines - 1] = last + "…";
  }
  return lines.join("\n");
}

/** Map a single Coach Protocol event to a lens view, or null if nothing to show. */
export function renderEvent(ev: CoachEvent, cfg: DisplayConfig): GlassesView | null {
  switch (ev.type) {
    case "prompt": {
      const e = ev as Extract<CoachEvent, { type: "prompt" }>;
      return {
        kind: "reference",
        title: e.kind.toUpperCase(),
        body: paginate(e.text, cfg.maxCharsPerLine, cfg.maxLines - 1),
        durationMs: e.ttl_ms,
      };
    }
    case "transcript.final": {
      if (!cfg.showTranscript) return null;
      const e = ev as Extract<CoachEvent, { type: "transcript.final" }>;
      const who = e.speaker.startsWith("me") ? "you" : "them";
      return { kind: "text", body: paginate(`${who}: ${e.text}`, cfg.maxCharsPerLine, cfg.maxLines) };
    }
    // partials and state are too chatty / not user-facing for a tiny lens.
    default:
      return null;
  }
}

/**
 * Respects the MentraOS display rate limit (1 update / ~300 ms) and prefers
 * prompts over transcript. Pull-based so it's testable with an injected clock:
 * push() stores the latest view; flush(nowMs) emits at most one per interval.
 */
export class DisplayCoalescer {
  private pending: GlassesView | null = null;
  private pendingIsPrompt = false;
  private lastEmitMs = -1e9;
  constructor(private minIntervalMs = 350) {}

  push(view: GlassesView | null, isPrompt: boolean): void {
    if (!view) return;
    // a prompt always replaces pending; a transcript only replaces a non-prompt
    if (isPrompt || !this.pendingIsPrompt) {
      this.pending = view;
      this.pendingIsPrompt = isPrompt;
    }
  }

  flush(nowMs: number): GlassesView | null {
    if (this.pending && nowMs - this.lastEmitMs >= this.minIntervalMs) {
      const v = this.pending;
      this.pending = null;
      this.pendingIsPrompt = false;
      this.lastEmitMs = nowMs;
      return v;
    }
    return null;
  }

  hasPending(): boolean {
    return this.pending !== null;
  }
}
