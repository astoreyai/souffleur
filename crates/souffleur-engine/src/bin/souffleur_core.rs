//! `souffleur-core` — the Souffleur core daemon.
//!
//! Captures audio (mic = me, system-audio monitor = them), transcribes each
//! channel with overlapping streaming windows, asks a local LLM for short
//! coaching cues, and publishes Coach Protocol events over a WebSocket for any
//! surface (phone PWA, desktop overlay, XREAL display) to consume.
//!
//! Modes (--mode): mic | monitor | duplex (default) | wav <path>
//!
//! Privacy: binds 127.0.0.1 by default. A non-loopback bind requires BOTH
//! `--listen-lan` and a shared secret (`--token` / $SOUFFLEUR_TOKEN), because the
//! stream carries live both-sides transcript + coaching prompts.

use anyhow::{anyhow, bail, Context, Result};
use crossbeam_channel::{Receiver, RecvTimeoutError};
use futures_util::{SinkExt, StreamExt};
use souffleur_engine::source;
use souffleur_engine::stream::{StreamConfig, StreamingStt};
use souffleur_engine::stt::Stt;
use souffleur_engine::suggest::{backend_is_cloud, make_backend, SuggestConfig, SuggestionEngine};
use souffleur_protocol::{Control, Event, Speaker, PROTOCOL_VERSION};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, Notify};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone)]
enum Mode {
    Mic,
    Monitor,
    Duplex,
    Wav(String),
}

#[derive(Clone)]
struct Config {
    mode: Mode,
    model: String,
    bind: String,
    threads: i32,
    monitor: Option<String>,
    duration_s: Option<u64>,
    print_stdout: bool,
    once: bool,
    no_suggest: bool,
    suggest_backend: String,
    suggest_model: Option<String>,
    wait_surface: bool,
    token: Option<String>,
}

/// Shared daemon state read by the heartbeat and written by capture/consent paths.
struct Shared {
    surfaces: AtomicUsize,
    active_channels: AtomicUsize,
    capturing: AtomicBool,
    consent_disclosed: AtomicBool,
    shutdown: AtomicBool,
}

/// Is `bind` a loopback address (safe to expose without auth)?
fn is_loopback_bind(bind: &str) -> bool {
    let host = bind.rsplit_once(':').map(|(h, _)| h).unwrap_or(bind);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host == "localhost" {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

fn parse_args() -> Result<Option<Config>> {
    let mut args = std::env::args().skip(1);
    let mut mode_kind = "duplex".to_string();
    let mut wav: Option<String> = None;
    let mut model = "models/ggml-base.en.bin".to_string();
    let mut bind = "127.0.0.1:8123".to_string();
    let mut threads = 8i32;
    let mut monitor: Option<String> = None;
    let mut duration_s: Option<u64> = None;
    let mut print_stdout = false;
    let mut once = false;
    let mut no_suggest = false;
    let mut suggest_backend = "local".to_string();
    let mut suggest_model: Option<String> = None;
    let mut allow_cloud = false;
    let mut wait_surface = false;
    let mut listen_lan = false;
    let mut token: Option<String> = std::env::var("SOUFFLEUR_TOKEN")
        .ok()
        .filter(|s| !s.is_empty());

    while let Some(a) = args.next() {
        match a.as_str() {
            "--list-devices" => {
                println!("input devices:");
                for d in souffleur_engine::audio::list_input_devices()? {
                    println!("  - {d}");
                }
                return Ok(None);
            }
            "--mode" => mode_kind = args.next().context("--mode")?,
            "--wav" => wav = Some(args.next().context("--wav")?),
            "--model" => model = args.next().context("--model")?,
            "--bind" => bind = args.next().context("--bind")?,
            "--threads" => threads = args.next().context("--threads")?.parse()?,
            "--monitor" => monitor = Some(args.next().context("--monitor")?),
            "--duration-s" => duration_s = Some(args.next().context("--duration-s")?.parse()?),
            "--print-stdout" => print_stdout = true,
            "--once" => once = true,
            "--no-suggest" => no_suggest = true,
            "--suggest-backend" => suggest_backend = args.next().context("--suggest-backend")?,
            "--suggest-model" => suggest_model = Some(args.next().context("--suggest-model")?),
            "--allow-cloud" => allow_cloud = true,
            "--wait-surface" => wait_surface = true,
            "--listen-lan" => listen_lan = true,
            "--token" => token = Some(args.next().context("--token")?),
            other => return Err(anyhow!("unknown arg: {other}")),
        }
    }

    let mode = match mode_kind.as_str() {
        "mic" => Mode::Mic,
        "monitor" => Mode::Monitor,
        "duplex" => Mode::Duplex,
        "wav" => Mode::Wav(wav.context("--mode wav requires --wav <path>")?),
        other => return Err(anyhow!("unknown --mode: {other}")),
    };

    // Privacy gate: a non-loopback bind exposes live transcript on the network.
    if !is_loopback_bind(&bind) {
        if !listen_lan {
            bail!(
                "refusing to bind non-loopback address {bind} without --listen-lan.\n\
                 The Coach Protocol stream carries live both-sides transcript + coaching prompts.\n\
                 Pass --listen-lan to opt in (and set --token / $SOUFFLEUR_TOKEN)."
            );
        }
        if token.is_none() {
            bail!(
                "binding {bind} (non-loopback) requires a shared secret.\n\
                 Set --token <secret> or $SOUFFLEUR_TOKEN; surfaces connect with ws://HOST:PORT/?token=<secret>."
            );
        }
    }

    // Cloud gate: a cloud suggestion backend sends the transcript off-device.
    if !no_suggest && backend_is_cloud(&suggest_backend) && !allow_cloud {
        bail!(
            "--suggest-backend {suggest_backend} is a CLOUD backend — it sends the live transcript off this machine.\n\
             Re-run with --allow-cloud to opt in. The transcript leaves the device and may be subject to two-party\n\
             consent / wiretapping law in your jurisdiction (CIPA/BIPA in 11 US states); coaching stays gated on a\n\
             disclosed-consent toggle. Local (on-device) is the default; pass nothing for it."
        );
    }

    Ok(Some(Config {
        mode,
        model,
        bind,
        threads,
        monitor,
        duration_s,
        print_stdout,
        once,
        no_suggest,
        suggest_backend,
        suggest_model,
        wait_surface,
        token,
    }))
}

fn now_ms(t0: Instant) -> u64 {
    t0.elapsed().as_millis() as u64
}

fn emit(tx: &broadcast::Sender<String>, print: bool, ev: &Event) {
    if let Ok(line) = ev.to_ndjson() {
        if print {
            println!("{line}");
        }
        let _ = tx.send(line);
    }
}

/// A capture channel: its speaker, audio stream, and optional device-alive flag.
type Channel = (Speaker, Receiver<Vec<f32>>, Option<Arc<AtomicBool>>);

/// Build the capture channels for the configured mode.
fn build_channels(cfg: &Config) -> Result<Vec<Channel>> {
    Ok(match &cfg.mode {
        Mode::Mic => {
            let (rx, alive) = source::spawn_mic()?;
            vec![(Speaker::Me, rx, Some(alive))]
        }
        Mode::Monitor => vec![(
            Speaker::Them,
            source::spawn_monitor(cfg.monitor.clone())?,
            None,
        )],
        Mode::Wav(p) => vec![(Speaker::Them, source::spawn_wav(p, 100)?, None)],
        Mode::Duplex => {
            let (mrx, alive) = source::spawn_mic()?;
            vec![
                (Speaker::Me, mrx, Some(alive)),
                (
                    Speaker::Them,
                    source::spawn_monitor(cfg.monitor.clone())?,
                    None,
                ),
            ]
        }
    })
}

/// Per-channel loop: pull audio -> streaming STT -> emit; forward finals to the
/// suggestion worker. Exits when the source ends OR a shutdown is requested, then
/// flushes. The last channel to exit flips `capturing` false and (if --once)
/// signals shutdown.
#[allow(clippy::too_many_arguments)]
fn channel_loop(
    rx: Receiver<Vec<f32>>,
    mut streamer: StreamingStt,
    tx: broadcast::Sender<String>,
    sug_tx: crossbeam_channel::Sender<(String, String)>,
    t0: Instant,
    print: bool,
    shared: Arc<Shared>,
    notify: Arc<Notify>,
    once: bool,
) {
    let handle = |ev: &Event, tx: &broadcast::Sender<String>| {
        if let Event::TranscriptFinal { speaker, text, .. } = ev {
            let _ = sug_tx.send((speaker.to_string(), text.clone()));
        }
        emit(tx, print, ev);
    };
    loop {
        if shared.shutdown.load(Ordering::Relaxed) {
            break;
        }
        match rx.recv_timeout(Duration::from_millis(150)) {
            Ok(chunk) => {
                let session_ms = now_ms(t0);
                for ev in streamer.push(&chunk, session_ms) {
                    handle(&ev, &tx);
                }
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break, // source ended
        }
    }
    for ev in streamer.flush(now_ms(t0)) {
        handle(&ev, &tx);
    }
    eprintln!("[channel:{}] ended", streamer.speaker());

    let remaining = shared.active_channels.fetch_sub(1, Ordering::Relaxed) - 1;
    if remaining == 0 {
        shared.capturing.store(false, Ordering::Relaxed);
        if once {
            shared.shutdown.store(true, Ordering::Relaxed);
            notify.notify_one();
        }
    }
}

/// Suggestion worker: debounce confirmed turns, ask the LLM, emit prompt events.
/// Resilient: on repeated failure it emits one Event::Error (so surfaces know
/// coaching went down) and periodically re-checks so it can recover.
fn suggestion_worker(
    mut engine: SuggestionEngine,
    rx: crossbeam_channel::Receiver<(String, String)>,
    tx: broadcast::Sender<String>,
    t0: Instant,
    print: bool,
    shared: Arc<Shared>,
) {
    let cloud = engine.is_cloud();
    let mut degraded = false;
    let mut fails = 0u32;
    let mut consent_warned = false;
    while let Ok((sp, txt)) = rx.recv() {
        engine.push_turn(&sp, &txt);
        while let Ok((s2, t2)) = rx.try_recv() {
            engine.push_turn(&s2, &t2);
        }

        let consent = shared.consent_disclosed.load(Ordering::Relaxed);
        // A cloud backend must not transmit the transcript until consent is disclosed.
        if cloud && !consent {
            if !consent_warned {
                consent_warned = true;
                emit(
                    &tx,
                    print,
                    &Event::Error {
                        version: PROTOCOL_VERSION,
                        t: now_ms(t0),
                        code: "cloud_consent_required".into(),
                        message: "cloud coaching is paused until you disclose the assistant (toggle consent)".into(),
                        fatal: false,
                    },
                );
            }
            continue;
        }
        consent_warned = false;

        // If we were degraded, re-check liveness before spending a full timeout.
        if degraded && engine.check().is_ok() {
            degraded = false;
            fails = 0;
            eprintln!("[suggest] recovered");
        }
        match engine.suggest_gated(now_ms(t0), consent) {
            Ok((evs, lat)) => {
                if degraded {
                    degraded = false;
                    fails = 0;
                }
                eprintln!("[suggest] {} prompt(s) in {lat} ms", evs.len());
                for ev in &evs {
                    emit(&tx, print, ev);
                }
            }
            Err(e) => {
                fails += 1;
                eprintln!("[suggest] error ({fails}): {e:#}");
                if !degraded && fails >= 2 {
                    degraded = true;
                    emit(
                        &tx,
                        print,
                        &Event::Error {
                            version: PROTOCOL_VERSION,
                            t: now_ms(t0),
                            code: "suggest_unavailable".into(),
                            message: "coaching suggestions are temporarily unavailable".into(),
                            fatal: false,
                        },
                    );
                }
            }
        }
    }
}

fn query_token(q: &str) -> Option<&str> {
    q.split('&').find_map(|p| p.strip_prefix("token="))
}

// The WS-accept callback must return `Result<Response, ErrorResponse>` — the
// shape tungstenite's `accept_hdr_async` dictates — so the large Err variant is
// not ours to box away.
#[allow(clippy::result_large_err)]
async fn handle_client(
    stream: tokio::net::TcpStream,
    tx: broadcast::Sender<String>,
    shared: Arc<Shared>,
    token: Option<Arc<String>>,
) {
    let peer = stream
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_default();
    let ws = if let Some(expected) = token.clone() {
        let cb = move |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
            let ok = req
                .uri()
                .query()
                .and_then(query_token)
                .map(|t| t == expected.as_str())
                .unwrap_or(false);
            if ok {
                Ok(resp)
            } else {
                Err(Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .body(Some("missing or invalid token".to_string()))
                    .unwrap())
            }
        };
        tokio_tungstenite::accept_hdr_async(stream, cb).await
    } else {
        tokio_tungstenite::accept_async(stream).await
    };
    let ws = match ws {
        Ok(w) => w,
        Err(e) => {
            eprintln!("[ws] handshake rejected ({peer}): {e}");
            return;
        }
    };
    let count = shared.surfaces.fetch_add(1, Ordering::Relaxed) + 1;
    eprintln!("[ws] surface connected: {peer} (surfaces={count})");
    let (mut write, mut read) = ws.split();
    let mut rx = tx.subscribe();

    let fwd_peer = peer.clone();
    let forward = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(line) => {
                    if write.send(Message::Text(line)).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("[ws] surface {fwd_peer} lagged, dropped {n} frame(s)");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Read the uplink: honor Control::SetConsent (the privacy disclosure toggle).
    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(t)) => {
                if let Ok(ctrl) = serde_json::from_str::<Control>(&t) {
                    match ctrl {
                        Control::SetConsent { disclosed } => {
                            shared.consent_disclosed.store(disclosed, Ordering::Relaxed);
                            eprintln!("[ws] {peer} set consent disclosed={disclosed}");
                        }
                        Control::Hint { text } => eprintln!("[ws] {peer} hint: {text}"),
                        Control::Dismiss { prompt_id } => {
                            eprintln!("[ws] {peer} dismiss {prompt_id}")
                        }
                        Control::Ack => {}
                    }
                }
            }
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => {}
        }
    }
    forward.abort();
    let count = shared
        .surfaces
        .fetch_sub(1, Ordering::Relaxed)
        .saturating_sub(1);
    eprintln!("[ws] surface disconnected: {peer} (surfaces={count})");
}

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = match parse_args()? {
        Some(c) => c,
        None => return Ok(()),
    };

    eprintln!("[core] loading model {} ...", cfg.model);
    let stt = Arc::new(Stt::load(&cfg.model, cfg.threads)?);
    let model_label = stt.model_label().to_string();
    let t0 = Instant::now();
    let (tx, _keepalive) = broadcast::channel::<String>(2048);
    let shared = Arc::new(Shared {
        surfaces: AtomicUsize::new(0),
        active_channels: AtomicUsize::new(0),
        capturing: AtomicBool::new(false),
        consent_disclosed: AtomicBool::new(false),
        shutdown: AtomicBool::new(false),
    });
    let notify = Arc::new(Notify::new());
    let token = cfg.token.clone().map(Arc::new);

    // Suggestion worker (honest degradation: if Ollama/model is unavailable, run
    // transcript-only rather than fabricating prompts).
    let (sug_tx, sug_rx) = crossbeam_channel::unbounded::<(String, String)>();
    let mut suggest_handle: Option<std::thread::JoinHandle<()>> = None;
    if cfg.no_suggest {
        eprintln!("[suggest] disabled (--no-suggest)");
    } else {
        match make_backend(&cfg.suggest_backend, cfg.suggest_model.clone()) {
            Ok(backend) => {
                let engine = SuggestionEngine::new(backend, SuggestConfig::default());
                let bname = engine.backend_name().to_string();
                let is_cloud = engine.is_cloud();
                match engine.check() {
                    Ok(()) => {
                        match engine.warmup() {
                            Ok(ms) => {
                                eprintln!("[suggest] backend {bname} ready (warm in {ms} ms)")
                            }
                            Err(e) => eprintln!("[suggest] backend {bname} ready (warmup: {e:#})"),
                        }
                        if is_cloud {
                            eprintln!("[suggest] CLOUD backend — transcript is sent off-device ONLY after consent is disclosed");
                        }
                        let tx = tx.clone();
                        let print = cfg.print_stdout;
                        let shared = shared.clone();
                        suggest_handle = Some(
                            std::thread::Builder::new()
                                .name("souffleur-suggest".into())
                                .spawn(move || {
                                    suggestion_worker(engine, sug_rx, tx, t0, print, shared)
                                })
                                .context("spawn suggestion worker")?,
                        );
                    }
                    Err(e) => eprintln!("[suggest] disabled: {e:#}  (transcript-only)"),
                }
            }
            Err(e) => eprintln!("[suggest] disabled: {e:#}  (transcript-only)"),
        }
    }

    // Bind + accept connections first, so surfaces can attach before capture.
    let listener = TcpListener::bind(&cfg.bind)
        .await
        .with_context(|| format!("bind {}", cfg.bind))?;
    eprintln!("[core] Coach Protocol WS listening on ws://{}", cfg.bind);
    if !is_loopback_bind(&cfg.bind) {
        eprintln!("[core] WARNING: bound to a non-loopback address; token auth REQUIRED, transcript leaves this host on the LAN");
    }
    {
        let tx = tx.clone();
        let shared = shared.clone();
        let token = token.clone();
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        tokio::spawn(handle_client(
                            stream,
                            tx.clone(),
                            shared.clone(),
                            token.clone(),
                        ));
                    }
                    Err(e) => {
                        eprintln!("[ws] accept error: {e}");
                        break;
                    }
                }
            }
        });
    }

    // Optionally hold capture until a surface is watching (don't coach to nobody).
    if cfg.wait_surface {
        eprintln!("[core] waiting for a surface to connect...");
        let deadline = Instant::now() + Duration::from_secs(60);
        while shared.surfaces.load(Ordering::Relaxed) == 0 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        eprintln!(
            "[core] {} surface(s) connected; starting capture",
            shared.surfaces.load(Ordering::Relaxed)
        );
    }

    // Capture + streaming threads, one per channel.
    let channels = build_channels(&cfg)?;
    shared
        .active_channels
        .store(channels.len(), Ordering::Relaxed);
    shared.capturing.store(true, Ordering::Relaxed);
    let mut handles = Vec::new();
    let mut alive_flags: Vec<Arc<AtomicBool>> = Vec::new();
    for (speaker, rx, alive) in channels {
        if let Some(a) = alive {
            alive_flags.push(a);
        }
        let streamer = StreamingStt::new(stt.clone(), speaker, StreamConfig::default());
        let tx = tx.clone();
        let sug_tx = sug_tx.clone();
        let print = cfg.print_stdout;
        let shared = shared.clone();
        let notify = notify.clone();
        let once = cfg.once;
        let h = std::thread::Builder::new()
            .name("souffleur-channel".into())
            .spawn(move || channel_loop(rx, streamer, tx, sug_tx, t0, print, shared, notify, once))
            .context("spawn channel loop")?;
        handles.push(h);
    }
    drop(sug_tx); // main keeps no sender; the worker ends once all channels drop theirs

    // State heartbeat: real capturing + consent, model, surface count.
    {
        let tx = tx.clone();
        let shared = shared.clone();
        let model = model_label.clone();
        tokio::spawn(async move {
            loop {
                let ev = Event::State {
                    version: PROTOCOL_VERSION,
                    t: now_ms(t0),
                    capturing: shared.capturing.load(Ordering::Relaxed),
                    model: model.clone(),
                    e2e_latency_ms: None,
                    consent_disclosed: shared.consent_disclosed.load(Ordering::Relaxed),
                    surfaces: shared.surfaces.load(Ordering::Relaxed) as u32,
                };
                if let Ok(line) = ev.to_ndjson() {
                    let _ = tx.send(line);
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        });
    }

    // Device-loss watcher: if a mic stream errors out, surface it once.
    if !alive_flags.is_empty() {
        let tx = tx.clone();
        let print = cfg.print_stdout;
        tokio::spawn(async move {
            let mut reported = false;
            loop {
                if !reported && alive_flags.iter().any(|a| !a.load(Ordering::Relaxed)) {
                    reported = true;
                    emit(
                        &tx,
                        print,
                        &Event::Error {
                            version: PROTOCOL_VERSION,
                            t: now_ms(t0),
                            code: "audio_device_lost".into(),
                            message: "an input device stopped delivering audio".into(),
                            fatal: false,
                        },
                    );
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        });
    }

    // Shutdown triggers, all funnelled through `notify` + the shutdown flag.
    if let Some(secs) = cfg.duration_s {
        let shared = shared.clone();
        let notify = notify.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(secs)).await;
            eprintln!("[core] duration {secs}s elapsed; shutting down");
            shared.shutdown.store(true, Ordering::Relaxed);
            notify.notify_one();
        });
    }
    {
        let shared = shared.clone();
        let notify = notify.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                eprintln!("[core] ctrl-c; shutting down");
                shared.shutdown.store(true, Ordering::Relaxed);
                notify.notify_one();
            }
        });
    }

    // Wait for shutdown, then drain gracefully.
    notify.notified().await;
    shared.shutdown.store(true, Ordering::Relaxed);
    // Channel threads observe the flag, flush, and exit; join them.
    for h in handles {
        let _ = h.join();
    }
    // Channels dropped their sug_tx clones; the worker's recv() now ends — join it.
    if let Some(h) = suggest_handle {
        let _ = h.join();
    }
    // Tell surfaces capture stopped, then give the WS forwarders a moment to flush.
    shared.capturing.store(false, Ordering::Relaxed);
    emit(
        &tx,
        cfg.print_stdout,
        &Event::State {
            version: PROTOCOL_VERSION,
            t: now_ms(t0),
            capturing: false,
            model: model_label,
            e2e_latency_ms: None,
            consent_disclosed: shared.consent_disclosed.load(Ordering::Relaxed),
            surfaces: shared.surfaces.load(Ordering::Relaxed) as u32,
        },
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    eprintln!("[core] shutdown complete");
    Ok(())
}
