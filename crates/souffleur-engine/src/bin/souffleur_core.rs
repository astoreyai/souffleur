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
    corpus: Option<String>,
    embed_model: String,
    retrieve_k: usize,
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
    let mut corpus: Option<String> = None;
    let mut embed_model = "nomic-embed-text".to_string();
    let mut retrieve_k: usize = 3;
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
            "--corpus" => corpus = Some(args.next().context("--corpus")?),
            "--embed-model" => embed_model = args.next().context("--embed-model")?,
            "--retrieve-k" => {
                retrieve_k = args
                    .next()
                    .context("--retrieve-k")?
                    .parse()
                    .context("--retrieve-k must be a positive integer")?
            }
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
        corpus,
        embed_model,
        retrieve_k,
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

/// (command-sender that channels/surfaces push into, suggestion-worker handle).
type SuggestionWorker = (
    crossbeam_channel::Sender<WorkerMsg>,
    Option<std::thread::JoinHandle<()>>,
);

/// (per-channel capture join handles, device-alive flags for the loss watcher).
type CaptureHandles = (Vec<std::thread::JoinHandle<()>>, Vec<Arc<AtomicBool>>);

/// Build the capture channels for the configured mode.
fn build_channels(cfg: &Config) -> Result<Vec<Channel>> {
    use source::{AudioSource, MicSource, MonitorSource, WavSource};
    let mic = || Box::new(MicSource) as Box<dyn AudioSource>;
    let monitor = || {
        Box::new(MonitorSource {
            name: cfg.monitor.clone(),
        }) as Box<dyn AudioSource>
    };
    // Each mode is a list of (speaker, source); the source decides its own alive flag.
    let plan: Vec<(Speaker, Box<dyn AudioSource>)> = match &cfg.mode {
        Mode::Mic => vec![(Speaker::Me, mic())],
        Mode::Monitor => vec![(Speaker::Them, monitor())],
        Mode::Wav(p) => vec![(
            Speaker::Them,
            Box::new(WavSource {
                path: p.clone(),
                chunk_ms: 100,
            }),
        )],
        Mode::Duplex => vec![(Speaker::Me, mic()), (Speaker::Them, monitor())],
    };
    let mut channels = Vec::with_capacity(plan.len());
    for (speaker, src) in plan {
        let (rx, alive) = src.spawn()?;
        channels.push((speaker, rx, alive));
    }
    Ok(channels)
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
    sug_tx: crossbeam_channel::Sender<WorkerMsg>,
    t0: Instant,
    print: bool,
    shared: Arc<Shared>,
    notify: Arc<Notify>,
    once: bool,
) {
    let handle = |ev: &Event, tx: &broadcast::Sender<String>| {
        if let Event::TranscriptFinal { speaker, text, .. } = ev {
            let _ = sug_tx.send(WorkerMsg::Turn(speaker.to_string(), text.clone()));
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

/// A message to the suggestion worker: a confirmed transcript turn, or a request
/// to (re)load the retrieval corpus from a host directory.
enum WorkerMsg {
    Turn(String, String),
    LoadCorpus(String),
}

/// Ingest a corpus directory into the engine, emitting `CorpusLoaded` on success
/// or a non-fatal `Error` on failure (so a bad path from a surface never crashes
/// the daemon).
fn load_corpus_into(
    engine: &mut SuggestionEngine,
    path: &str,
    embed_model: &str,
    retrieve_k: usize,
    tx: &broadcast::Sender<String>,
    print: bool,
    t0: Instant,
) {
    match souffleur_engine::corpus::Corpus::ingest_model(std::path::Path::new(path), embed_model) {
        Ok(corpus) => {
            let (chunks, sources) = (corpus.len() as u32, corpus.sources() as u32);
            engine.set_corpus(corpus);
            engine.set_retrieve_k(retrieve_k);
            eprintln!(
                "[corpus] loaded {chunks} chunks from {sources} files via {embed_model} ({path})"
            );
            emit(
                tx,
                print,
                &Event::CorpusLoaded {
                    version: PROTOCOL_VERSION,
                    t: now_ms(t0),
                    path: path.to_string(),
                    chunks,
                    sources,
                },
            );
        }
        Err(e) => {
            eprintln!("[corpus] load failed: {e:#}");
            emit(
                tx,
                print,
                &Event::Error {
                    version: PROTOCOL_VERSION,
                    t: now_ms(t0),
                    code: "corpus_load_failed".into(),
                    message: format!("could not load corpus {path}: {e}"),
                    fatal: false,
                },
            );
        }
    }
}

/// Suggestion worker: debounce confirmed turns, ask the LLM, emit prompt events,
/// and apply runtime corpus loads. Shutdown-aware (a lingering command sender
/// can't keep it alive). Resilient: on repeated failure it emits one Event::Error
/// (so surfaces know coaching went down) and periodically re-checks to recover.
#[allow(clippy::too_many_arguments)]
fn suggestion_worker(
    mut engine: SuggestionEngine,
    rx: crossbeam_channel::Receiver<WorkerMsg>,
    tx: broadcast::Sender<String>,
    t0: Instant,
    print: bool,
    shared: Arc<Shared>,
    embed_model: String,
    retrieve_k: usize,
) {
    let cloud = engine.is_cloud();
    let mut degraded = false;
    let mut fails = 0u32;
    let mut consent_warned = false;
    loop {
        if shared.shutdown.load(Ordering::Relaxed) {
            break;
        }
        let msg = match rx.recv_timeout(Duration::from_millis(150)) {
            Ok(m) => m,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        match msg {
            WorkerMsg::LoadCorpus(path) => {
                load_corpus_into(&mut engine, &path, &embed_model, retrieve_k, &tx, print, t0);
                continue;
            }
            WorkerMsg::Turn(sp, txt) => engine.push_turn(&sp, &txt),
        }
        // Drain queued messages: batch turns (debounce) and apply any corpus load.
        loop {
            match rx.try_recv() {
                Ok(WorkerMsg::Turn(s2, t2)) => engine.push_turn(&s2, &t2),
                Ok(WorkerMsg::LoadCorpus(path)) => {
                    load_corpus_into(&mut engine, &path, &embed_model, retrieve_k, &tx, print, t0)
                }
                Err(_) => break,
            }
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

/// Constant-time string equality for the LAN auth token, so a comparison's
/// duration does not leak how many leading bytes matched. (Length is allowed to
/// differ early — the token's length is not the secret, its bytes are.)
fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
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
    corpus_cmd: Option<crossbeam_channel::Sender<WorkerMsg>>,
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
                .map(|t| ct_eq(t, expected.as_str()))
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
                        Control::SetCorpus { path } => {
                            if let Some(cmd) = &corpus_cmd {
                                eprintln!("[ws] {peer} set_corpus {path}");
                                let _ = cmd.send(WorkerMsg::LoadCorpus(path));
                            } else {
                                eprintln!(
                                    "[ws] {peer} set_corpus ignored (suggestions disabled): {path}"
                                );
                            }
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

/// Coordinates graceful shutdown: any trigger flips the flag and wakes the drain.
#[derive(Clone)]
struct Shutdown {
    shared: Arc<Shared>,
    notify: Arc<Notify>,
}

impl Shutdown {
    /// Signal all loops to stop and wake the main drain.
    fn trigger(&self, reason: &str) {
        eprintln!("[core] {reason}; shutting down");
        self.shared.shutdown.store(true, Ordering::Relaxed);
        self.notify.notify_one();
    }
    /// Block until a shutdown is triggered, then confirm the flag.
    async fn wait(&self) {
        self.notify.notified().await;
        self.shared.shutdown.store(true, Ordering::Relaxed);
    }
}

/// Build the suggestion backend (+ optional corpus) and spawn the worker thread.
/// Returns the turn-sender (channels push confirmed turns into it) and the worker
/// handle. On any non-fatal failure it degrades to transcript-only: the sender is
/// still returned (channel_loop's sends become no-ops) and the handle is `None`.
/// A `--corpus` that cannot be ingested is fatal (propagated), since the user
/// asked for grounding and silent un-grounded coaching would be dishonest.
fn spawn_suggestion(
    cfg: &Config,
    tx: &broadcast::Sender<String>,
    t0: Instant,
    shared: &Arc<Shared>,
) -> Result<SuggestionWorker> {
    let (sug_tx, sug_rx) = crossbeam_channel::unbounded::<WorkerMsg>();
    if cfg.no_suggest {
        eprintln!("[suggest] disabled (--no-suggest)");
        return Ok((sug_tx, None));
    }
    let backend = match make_backend(&cfg.suggest_backend, cfg.suggest_model.clone()) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[suggest] disabled: {e:#}  (transcript-only)");
            return Ok((sug_tx, None));
        }
    };
    let mut engine = SuggestionEngine::new(backend, SuggestConfig::default());
    if let Some(dir) = &cfg.corpus {
        let corpus = souffleur_engine::corpus::Corpus::ingest_model(
            std::path::Path::new(dir),
            &cfg.embed_model,
        )
        .with_context(|| format!("ingest --corpus {dir}"))?;
        eprintln!(
            "[corpus] {} chunks from {} files in {dir} via {} (top-{} per cue)",
            corpus.len(),
            corpus.sources(),
            cfg.embed_model,
            cfg.retrieve_k
        );
        engine.set_corpus(corpus);
        engine.set_retrieve_k(cfg.retrieve_k);
    }
    let bname = engine.backend_name().to_string();
    let is_cloud = engine.is_cloud();
    if let Err(e) = engine.check() {
        eprintln!("[suggest] disabled: {e:#}  (transcript-only)");
        return Ok((sug_tx, None));
    }
    match engine.warmup() {
        Ok(ms) => eprintln!("[suggest] backend {bname} ready (warm in {ms} ms)"),
        Err(e) => eprintln!("[suggest] backend {bname} ready (warmup: {e:#})"),
    }
    if is_cloud {
        eprintln!("[suggest] CLOUD backend — transcript is sent off-device ONLY after consent is disclosed");
    }
    let tx = tx.clone();
    let print = cfg.print_stdout;
    let shared = shared.clone();
    let embed_model = cfg.embed_model.clone();
    let retrieve_k = cfg.retrieve_k;
    let handle = std::thread::Builder::new()
        .name("souffleur-suggest".into())
        .spawn(move || {
            suggestion_worker(
                engine,
                sug_rx,
                tx,
                t0,
                print,
                shared,
                embed_model,
                retrieve_k,
            )
        })
        .context("spawn suggestion worker")?;
    Ok((sug_tx, Some(handle)))
}

/// Spawn the accept loop: each TCP connection becomes a Coach Protocol client.
fn spawn_accept_loop(
    listener: TcpListener,
    tx: broadcast::Sender<String>,
    shared: Arc<Shared>,
    token: Option<Arc<String>>,
    corpus_cmd: Option<crossbeam_channel::Sender<WorkerMsg>>,
) {
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    tokio::spawn(handle_client(
                        stream,
                        tx.clone(),
                        shared.clone(),
                        token.clone(),
                        corpus_cmd.clone(),
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

/// Optionally hold capture until a surface connects (don't coach to nobody).
async fn wait_for_surface(shared: &Arc<Shared>) {
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

/// Spawn one capture+streaming thread per channel. Returns the join handles and
/// the device-alive flags (consumed by the device-loss watcher).
fn spawn_capture(
    cfg: &Config,
    stt: Arc<Stt>,
    tx: &broadcast::Sender<String>,
    sug_tx: &crossbeam_channel::Sender<WorkerMsg>,
    t0: Instant,
    shared: &Arc<Shared>,
    notify: &Arc<Notify>,
) -> Result<CaptureHandles> {
    let channels = build_channels(cfg)?;
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
    Ok((handles, alive_flags))
}

/// Spawn the state heartbeat: capturing + consent + model + surface count, every 3s.
fn spawn_heartbeat(
    tx: broadcast::Sender<String>,
    shared: Arc<Shared>,
    model_label: String,
    t0: Instant,
) {
    tokio::spawn(async move {
        loop {
            let ev = Event::State {
                version: PROTOCOL_VERSION,
                t: now_ms(t0),
                capturing: shared.capturing.load(Ordering::Relaxed),
                model: model_label.clone(),
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

/// If any input device stops delivering audio, emit one non-fatal Error event.
fn spawn_device_watcher(
    alive_flags: Vec<Arc<AtomicBool>>,
    tx: broadcast::Sender<String>,
    t0: Instant,
    print: bool,
) {
    if alive_flags.is_empty() {
        return;
    }
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

/// Wire the shutdown triggers (duration timeout, ctrl-c) into `shutdown`.
/// (The `--once` path triggers from inside `channel_loop` when the last channel ends.)
fn spawn_shutdown_triggers(cfg: &Config, shutdown: Shutdown) {
    if let Some(secs) = cfg.duration_s {
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(secs)).await;
            shutdown.trigger(&format!("duration {secs}s elapsed"));
        });
    }
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            shutdown.trigger("ctrl-c");
        }
    });
}

/// Wait for shutdown, join capture + suggestion threads, emit a final idle state,
/// then give the WS forwarders a moment to flush.
async fn drain(
    shutdown: &Shutdown,
    handles: Vec<std::thread::JoinHandle<()>>,
    suggest_handle: Option<std::thread::JoinHandle<()>>,
    tx: &broadcast::Sender<String>,
    model_label: String,
    t0: Instant,
    print: bool,
) {
    shutdown.wait().await;
    // Channel threads observe the flag, flush, and exit; join them.
    for h in handles {
        let _ = h.join();
    }
    // Channels dropped their sug_tx clones; the worker's recv() now ends — join it.
    if let Some(h) = suggest_handle {
        let _ = h.join();
    }
    let shared = &shutdown.shared;
    shared.capturing.store(false, Ordering::Relaxed);
    emit(
        tx,
        print,
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
    let shutdown = Shutdown {
        shared: shared.clone(),
        notify: notify.clone(),
    };
    let token = cfg.token.clone().map(Arc::new);

    let (sug_tx, suggest_handle) = spawn_suggestion(&cfg, &tx, t0, &shared)?;
    // Surfaces can load a corpus at runtime only if a worker exists to hold it.
    let corpus_cmd = suggest_handle.as_ref().map(|_| sug_tx.clone());

    // Bind + accept connections first, so surfaces can attach before capture.
    let listener = TcpListener::bind(&cfg.bind)
        .await
        .with_context(|| format!("bind {}", cfg.bind))?;
    eprintln!("[core] Coach Protocol WS listening on ws://{}", cfg.bind);
    if !is_loopback_bind(&cfg.bind) {
        eprintln!("[core] WARNING: bound to a non-loopback address; token auth REQUIRED, transcript leaves this host on the LAN");
    }
    spawn_accept_loop(listener, tx.clone(), shared.clone(), token, corpus_cmd);

    // Optionally hold capture until a surface is watching (don't coach to nobody).
    if cfg.wait_surface {
        wait_for_surface(&shared).await;
    }

    let (handles, alive_flags) = spawn_capture(&cfg, stt, &tx, &sug_tx, t0, &shared, &notify)?;
    drop(sug_tx); // main keeps no sender; the worker ends once all channels drop theirs

    spawn_heartbeat(tx.clone(), shared.clone(), model_label.clone(), t0);
    spawn_device_watcher(alive_flags, tx.clone(), t0, cfg.print_stdout);
    spawn_shutdown_triggers(&cfg, shutdown.clone());

    drain(
        &shutdown,
        handles,
        suggest_handle,
        &tx,
        model_label,
        t0,
        cfg.print_stdout,
    )
    .await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ct_eq_matches_and_rejects() {
        assert!(ct_eq("s3cr3t", "s3cr3t"));
        assert!(ct_eq("", ""));
        assert!(!ct_eq("s3cr3t", "s3cr3x")); // last byte differs
        assert!(!ct_eq("s3cr3t", "S3cr3t")); // case-sensitive
        assert!(!ct_eq("s3cr3t", "s3cr3")); // length differs
        assert!(!ct_eq("abc", "")); // empty vs non-empty
    }

    #[test]
    fn query_token_extracts() {
        assert_eq!(query_token("token=abc"), Some("abc"));
        assert_eq!(query_token("foo=1&token=abc"), Some("abc"));
        assert_eq!(query_token("token=abc&foo=1"), Some("abc"));
        assert_eq!(query_token("foo=1"), None);
        assert_eq!(query_token(""), None);
    }

    #[test]
    fn loopback_detection() {
        assert!(is_loopback_bind("127.0.0.1:8123"));
        assert!(is_loopback_bind("localhost:8123"));
        assert!(is_loopback_bind("[::1]:8123"));
        assert!(!is_loopback_bind("0.0.0.0:8123"));
        assert!(!is_loopback_bind("100.64.1.2:8123")); // a tailscale-style LAN addr
    }
}
