/**
 * Souffleur — MentraOS glasses surface.
 *
 * A thin surface: it connects to the Souffleur core's Coach Protocol WebSocket
 * and pushes prompts + transcript to the wearer's lens via the MentraOS SDK
 * (`session.layouts.*`). The core does all capture/STT/suggestion; the glasses
 * are just another display surface (like the phone and the desktop overlay).
 *
 * Running this end-to-end requires real hardware + a MentraOS Cloud app:
 *   - MentraOS-supported glasses (Even Realities G1/G2, Vuzix Z100, Mentra Mach1)
 *   - the MentraOS phone app paired to the glasses
 *   - an app registered at https://console.mentra.glass (packageName + apiKey)
 *   - this server reachable by MentraOS Cloud (public URL / tunnel)
 * The bridge logic (./bridge.ts) is hardware-independent and unit-tested.
 */

import { AppServer, AppSession } from "@mentra/sdk";
import WebSocket from "ws";
import {
  parseEvent,
  renderEvent,
  DisplayCoalescer,
  DEFAULT_DISPLAY,
  type DisplayConfig,
  type GlassesView,
} from "./bridge";

const SOUFFLEUR_WS = process.env.SOUFFLEUR_WS ?? "ws://127.0.0.1:8123";
const PACKAGE_NAME = process.env.MENTRAOS_PACKAGE ?? "space.souffleur.coach";
const API_KEY = process.env.MENTRAOS_API_KEY ?? "";
const PORT = Number.parseInt(process.env.PORT ?? "7010", 10);

const DISPLAY: DisplayConfig = {
  ...DEFAULT_DISPLAY,
  // Override from env once measured on the real glasses (see docs/research/04 §4).
  maxCharsPerLine: Number.parseInt(process.env.GLASSES_CHARS_PER_LINE ?? "", 10) || DEFAULT_DISPLAY.maxCharsPerLine,
  maxLines: Number.parseInt(process.env.GLASSES_LINES ?? "", 10) || DEFAULT_DISPLAY.maxLines,
};

class SouffleurGlasses extends AppServer {
  protected async onSession(session: AppSession, sessionId: string, _userId: string): Promise<void> {
    console.log(`[souffleur-glasses] session ${sessionId} -> bridging ${SOUFFLEUR_WS}`);
    const coalescer = new DisplayCoalescer(350);

    const show = (v: GlassesView): void => {
      if (v.kind === "reference") {
        session.layouts.showReferenceCard(v.title, v.body, v.durationMs ? { durationMs: v.durationMs } : undefined);
      } else {
        session.layouts.showTextWall(v.body, v.durationMs ? { durationMs: v.durationMs } : undefined);
      }
    };

    // pull from the coalescer at a steady tick (respects the ~300ms lens rate limit)
    const flushTimer = setInterval(() => {
      const v = coalescer.flush(Date.now());
      if (v) show(v);
    }, 120);

    let ws: WebSocket | null = null;
    let stopped = false;
    const connect = (): void => {
      if (stopped) return;
      ws = new WebSocket(SOUFFLEUR_WS);
      ws.on("open", () => session.layouts.showTextWall("Souffleur connected"));
      ws.on("message", (data: WebSocket.RawData) => {
        const ev = parseEvent(data.toString());
        if (!ev) return;
        coalescer.push(renderEvent(ev, DISPLAY), ev.type === "prompt");
      });
      ws.on("close", () => {
        if (!stopped) setTimeout(connect, 1500);
      });
      ws.on("error", () => ws?.close());
    };
    connect();

    this.addCleanupHandler(() => {
      stopped = true;
      clearInterval(flushTimer);
      ws?.close();
    });
  }
}

new SouffleurGlasses({ packageName: PACKAGE_NAME, apiKey: API_KEY, port: PORT })
  .start()
  .then(() => console.log(`[souffleur-glasses] AppServer on :${PORT}, bridging ${SOUFFLEUR_WS}`))
  .catch((e) => {
    console.error("[souffleur-glasses] failed to start:", e);
    process.exit(1);
  });
