//! `souffleur-core` — the Souffleur core daemon (Phase 1).
//!
//! Captures audio (mic = me, system-audio monitor = them), transcribes each
//! channel with overlapping streaming windows, asks a local LLM for short
//! coaching cues, and publishes Coach Protocol events over a localhost WebSocket
//! for any surface (phone PWA, glasses, overlay) to consume.
//!
//! Modes (--mode):
//!   mic       live default input only (speaker=me)
//!   monitor   live system-audio loopback via parec (speaker=them)
//!   duplex    mic + monitor together (the real both-sides) [default]
//!   wav       stream a real WAV in realtime (speaker=them; for tests)
//!
//! Flags: --model <path> --bind <addr> --threads <n> --wav <path>
//!        --monitor <src> --duration-s <n> --once --print-stdout
//!        --no-suggest --suggest-model <name>

use anyhow::{anyhow, Context, Result};
use crossbeam_channel::Receiver;
use futures_util::{SinkExt, StreamExt};
use souffleur_engine::stream::{StreamConfig, StreamingStt};
use souffleur_engine::stt::Stt;
use souffleur_engine::suggest::{SuggestConfig, SuggestionEngine};
use souffleur_engine::source;
use souffleur_protocol::{Event, Speaker, PROTOCOL_VERSION};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
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
    suggest_model: String,
    wait_surface: bool,
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
    let mut suggest_model = "qwen3:8b".to_string();
    let mut wait_surface = false;

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
            "--suggest-model" => suggest_model = args.next().context("--suggest-model")?,
            "--wait-surface" => wait_surface = true,
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
        suggest_model,
        wait_surface,
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

/// Build the (speaker, audio-stream) channels for the configured mode.
fn build_channels(cfg: &Config) -> Result<Vec<(Speaker, Receiver<Vec<f32>>)>> {
    Ok(match &cfg.mode {
        Mode::Mic => vec![(Speaker::Me, source::spawn_mic()?)],
        Mode::Monitor => vec![(Speaker::Them, source::spawn_monitor(cfg.monitor.clone())?)],
        Mode::Wav(p) => vec![(Speaker::Them, source::spawn_wav(p, 100)?)],
        Mode::Duplex => vec![
            (Speaker::Me, source::spawn_mic()?),
            (Speaker::Them, source::spawn_monitor(cfg.monitor.clone())?),
        ],
    })
}

/// Per-channel loop: pull audio -> streaming STT -> emit; forward finals to the
/// suggestion worker. Returns when the source ends (then flushes).
fn channel_loop(
    rx: Receiver<Vec<f32>>,
    mut streamer: StreamingStt,
    tx: broadcast::Sender<String>,
    sug_tx: crossbeam_channel::Sender<(String, String)>,
    t0: Instant,
    print: bool,
) {
    let handle = |ev: &Event, tx: &broadcast::Sender<String>| {
        if let Event::TranscriptFinal { speaker, text, .. } = ev {
            let _ = sug_tx.send((speaker.to_string(), text.clone()));
        }
        emit(tx, print, ev);
    };
    for chunk in rx.iter() {
        let session_ms = now_ms(t0);
        for ev in streamer.push(&chunk, session_ms) {
            handle(&ev, &tx);
        }
    }
    for ev in streamer.flush(now_ms(t0)) {
        handle(&ev, &tx);
    }
    eprintln!("[channel:{}] source ended", streamer.speaker());
}

/// Suggestion worker: debounce confirmed turns, ask the LLM, emit prompt events.
fn suggestion_worker(
    mut engine: SuggestionEngine,
    rx: crossbeam_channel::Receiver<(String, String)>,
    tx: broadcast::Sender<String>,
    t0: Instant,
    print: bool,
) {
    while let Ok((sp, txt)) = rx.recv() {
        engine.push_turn(&sp, &txt);
        // Coalesce any turns that arrived while we were idle.
        while let Ok((s2, t2)) = rx.try_recv() {
            engine.push_turn(&s2, &t2);
        }
        match engine.suggest(now_ms(t0)) {
            Ok((evs, lat)) => {
                eprintln!("[suggest] {} prompt(s) in {lat} ms", evs.len());
                for ev in &evs {
                    emit(&tx, print, ev);
                }
            }
            Err(e) => eprintln!("[suggest] error: {e:#}"),
        }
    }
}

async fn handle_client(
    stream: tokio::net::TcpStream,
    tx: broadcast::Sender<String>,
    surfaces: Arc<AtomicUsize>,
) {
    let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
    let ws = match tokio_tungstenite::accept_async(stream).await {
        Ok(w) => w,
        Err(e) => {
            eprintln!("[ws] handshake failed ({peer}): {e}");
            return;
        }
    };
    let count = surfaces.fetch_add(1, Ordering::Relaxed) + 1;
    eprintln!("[ws] surface connected: {peer} (surfaces={count})");
    let (mut write, mut read) = ws.split();
    let mut rx = tx.subscribe();

    let forward = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(line) => {
                    if write.send(Message::Text(line)).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => {}
        }
    }
    forward.abort();
    let count = surfaces.fetch_sub(1, Ordering::Relaxed).saturating_sub(1);
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
    let surfaces = Arc::new(AtomicUsize::new(0));

    // Suggestion worker (honest degradation: if Ollama/model is unavailable,
    // run transcript-only rather than fabricating prompts).
    let (sug_tx, sug_rx) = crossbeam_channel::unbounded::<(String, String)>();
    if cfg.no_suggest {
        eprintln!("[suggest] disabled (--no-suggest)");
    } else {
        let engine = SuggestionEngine::new(SuggestConfig {
            model: cfg.suggest_model.clone(),
            ..Default::default()
        });
        match engine.check() {
            Ok(()) => {
                match engine.warmup() {
                    Ok(ms) => eprintln!("[suggest] using local model {} (warm in {ms} ms)", cfg.suggest_model),
                    Err(e) => eprintln!("[suggest] using local model {} (warmup failed: {e:#})", cfg.suggest_model),
                }
                let tx = tx.clone();
                let print = cfg.print_stdout;
                std::thread::Builder::new()
                    .name("souffleur-suggest".into())
                    .spawn(move || suggestion_worker(engine, sug_rx, tx, t0, print))
                    .context("spawn suggestion worker")?;
            }
            Err(e) => eprintln!("[suggest] disabled: {e:#}  (transcript-only)"),
        }
    }

    // Bind + accept connections first, so surfaces can attach before capture.
    let listener = TcpListener::bind(&cfg.bind)
        .await
        .with_context(|| format!("bind {}", cfg.bind))?;
    eprintln!("[core] Coach Protocol WS listening on ws://{}", cfg.bind);
    let _ = PROTOCOL_VERSION;
    {
        let tx = tx.clone();
        let surfaces = surfaces.clone();
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        tokio::spawn(handle_client(stream, tx.clone(), surfaces.clone()));
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
        while surfaces.load(Ordering::Relaxed) == 0 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        eprintln!(
            "[core] {} surface(s) connected; starting capture",
            surfaces.load(Ordering::Relaxed)
        );
    }

    // Capture + streaming threads, one per channel.
    let channels = build_channels(&cfg)?;
    let mut handles = Vec::new();
    for (speaker, rx) in channels {
        let streamer = StreamingStt::new(stt.clone(), speaker, StreamConfig::default());
        let tx = tx.clone();
        let sug_tx = sug_tx.clone();
        let print = cfg.print_stdout;
        let h = std::thread::Builder::new()
            .name("souffleur-channel".into())
            .spawn(move || channel_loop(rx, streamer, tx, sug_tx, t0, print))
            .context("spawn channel loop")?;
        handles.push(h);
    }
    drop(sug_tx); // channels hold their own clones; this lets the worker end when they do

    // State heartbeat: drives the surface's status pill / model / surface count.
    {
        let tx = tx.clone();
        let surfaces = surfaces.clone();
        let model = model_label.clone();
        tokio::spawn(async move {
            loop {
                let ev = Event::State {
                    version: PROTOCOL_VERSION,
                    t: now_ms(t0),
                    capturing: true,
                    model: model.clone(),
                    e2e_latency_ms: None,
                    consent_disclosed: false,
                    surfaces: surfaces.load(Ordering::Relaxed) as u32,
                };
                if let Ok(line) = ev.to_ndjson() {
                    let _ = tx.send(line);
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        });
    }

    // Exit policy: --duration-s stops live modes; --once exits when sources end.
    if let Some(secs) = cfg.duration_s {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(secs));
            eprintln!("[core] duration {secs}s elapsed; exiting");
            std::process::exit(0);
        });
    }
    if cfg.once {
        std::thread::spawn(move || {
            for h in handles {
                let _ = h.join();
            }
            std::thread::sleep(Duration::from_millis(800)); // let suggestions flush
            eprintln!("[core] sources done (--once); exiting");
            std::process::exit(0);
        });
    }

    // Keep the runtime alive; --once/--duration exit via process::exit.
    std::future::pending::<()>().await;
    Ok(())
}
