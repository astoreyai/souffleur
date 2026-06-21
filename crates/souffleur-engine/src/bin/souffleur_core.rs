//! `souffleur-core` — the Phase 0 core daemon.
//!
//! Captures audio, transcribes it on-device in rolling windows, and emits Coach
//! Protocol events over a localhost WebSocket for any surface to consume.
//!
//! Sources:
//!   --source mic               live capture from the default input device (speaker=me)
//!   --source wav --wav <path>  stream a real WAV in realtime-paced windows (speaker=them)
//!
//! Other flags:
//!   --list-devices             print input devices and exit
//!   --model <path>             ggml/gguf model (default models/ggml-base.en.bin)
//!   --bind <addr>              default 127.0.0.1:8123 (localhost only; transcript is sensitive)
//!   --window-ms <n>            transcription window (default 3000)
//!   --threads <n>              whisper threads (default 8)
//!   --duration-s <n>           mic mode: stop after n seconds
//!   --print-stdout             also print every emitted frame to stdout
//!
//! Windowing here is non-overlapping (chunked). A true overlapping LocalAgreement
//! sliding window is the Phase 1 refinement; this is real chunked STT, not a stub.

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use souffleur_engine::{audio, resample, stt::Stt};
use souffleur_protocol::{Event, Speaker, PROTOCOL_VERSION};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone)]
enum Source {
    Mic,
    Wav(String),
}

#[derive(Clone)]
struct Config {
    source: Source,
    model: String,
    bind: String,
    window_ms: u64,
    speaker: Speaker,
    threads: i32,
    duration_s: Option<u64>,
    print_stdout: bool,
    once: bool,
}

fn parse_args() -> Result<Option<Config>> {
    let mut args = std::env::args().skip(1).peekable();
    let mut source_kind = "mic".to_string();
    let mut wav: Option<String> = None;
    let mut model = "models/ggml-base.en.bin".to_string();
    let mut bind = "127.0.0.1:8123".to_string();
    let mut window_ms = 3000u64;
    let mut threads = 8i32;
    let mut duration_s: Option<u64> = None;
    let mut print_stdout = false;
    let mut once = false;
    let mut speaker_opt: Option<Speaker> = None;

    while let Some(a) = args.next() {
        match a.as_str() {
            "--list-devices" => {
                println!("input devices:");
                for d in audio::list_input_devices()? {
                    println!("  - {d}");
                }
                return Ok(None);
            }
            "--source" => source_kind = args.next().context("--source needs a value")?,
            "--wav" => wav = Some(args.next().context("--wav needs a path")?),
            "--model" => model = args.next().context("--model needs a path")?,
            "--bind" => bind = args.next().context("--bind needs an addr")?,
            "--window-ms" => window_ms = args.next().context("--window-ms")?.parse()?,
            "--threads" => threads = args.next().context("--threads")?.parse()?,
            "--duration-s" => duration_s = Some(args.next().context("--duration-s")?.parse()?),
            "--speaker" => {
                speaker_opt = Some(
                    args.next()
                        .context("--speaker")?
                        .parse()
                        .map_err(|e| anyhow!("{e}"))?,
                )
            }
            "--print-stdout" => print_stdout = true,
            "--once" => once = true,
            other => return Err(anyhow!("unknown arg: {other}")),
        }
    }

    let source = match source_kind.as_str() {
        "mic" => Source::Mic,
        "wav" => Source::Wav(wav.context("--source wav requires --wav <path>")?),
        other => return Err(anyhow!("unknown --source: {other}")),
    };
    // Default speaker: mic = me, wav (stand-in for the remote side) = them.
    let speaker = speaker_opt.unwrap_or(match source {
        Source::Mic => Speaker::Me,
        Source::Wav(_) => Speaker::Them,
    });

    Ok(Some(Config {
        source,
        model,
        bind,
        window_ms,
        speaker,
        threads,
        duration_s,
        print_stdout,
        once,
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
        let _ = tx.send(line); // Err only means no subscribers; fine.
    }
}

fn state(t0: Instant, model: &str, capturing: bool, surfaces: u32) -> Event {
    Event::State {
        version: PROTOCOL_VERSION,
        t: now_ms(t0),
        capturing,
        model: model.to_string(),
        e2e_latency_ms: None,
        consent_disclosed: false,
        surfaces,
    }
}

/// Blocking capture+STT loop. Runs on a dedicated std thread (whisper is CPU-blocking).
fn capture_loop(
    cfg: Config,
    stt: Stt,
    tx: broadcast::Sender<String>,
    t0: Instant,
    surfaces: Arc<AtomicUsize>,
) -> Result<()> {
    let model_label = stt.model_label().to_string();

    // For the file source, wait (up to 10s) for a surface to connect so it sees
    // the whole stream from the first window. A live mic never waits.
    if matches!(cfg.source, Source::Wav(_)) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while surfaces.load(Ordering::Relaxed) == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        eprintln!(
            "[capture] {} surface(s) connected; streaming",
            surfaces.load(Ordering::Relaxed)
        );
    }

    let n = surfaces.load(Ordering::Relaxed) as u32;
    emit(&tx, cfg.print_stdout, &state(t0, &model_label, true, n));

    match &cfg.source {
        Source::Wav(path) => stream_wav(path, &cfg, &stt, &tx, t0)?,
        Source::Mic => stream_mic(&cfg, &stt, &tx, t0)?,
    }

    let n = surfaces.load(Ordering::Relaxed) as u32;
    emit(&tx, cfg.print_stdout, &state(t0, &model_label, false, n));
    eprintln!("[capture] done");

    if cfg.once {
        // Let the WS forward task flush the final frames, then exit the process
        // so connected surfaces see a clean socket close.
        std::thread::sleep(Duration::from_millis(300));
        std::process::exit(0);
    }
    Ok(())
}

fn stream_wav(
    path: &str,
    cfg: &Config,
    stt: &Stt,
    tx: &broadcast::Sender<String>,
    t0: Instant,
) -> Result<()> {
    let mut reader = hound::WavReader::open(path).with_context(|| format!("open wav {path}"))?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<std::result::Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let max = ((1i64 << (spec.bits_per_sample - 1)) - 1) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / max))
                .collect::<std::result::Result<_, _>>()?
        }
    };
    let mono = resample::to_mono(&samples, spec.channels as usize);
    let audio16 = resample::resample_linear(&mono, spec.sample_rate, 16000);
    let window_samples = (cfg.window_ms as usize * 16000) / 1000;

    eprintln!(
        "[wav] {path}: {:.2}s audio, {} ms windows -> ~{} chunks",
        audio16.len() as f64 / 16000.0,
        cfg.window_ms,
        audio16.len().div_ceil(window_samples)
    );

    let mut utt = 0u32;
    for chunk in audio16.chunks(window_samples) {
        // Pace to realtime: the window "fills" over its own audio duration.
        std::thread::sleep(Duration::from_secs_f64(chunk.len() as f64 / 16000.0));
        let t_capture_end = now_ms(t0);
        let started = Instant::now();
        let out = stt.transcribe(chunk)?;
        let stt_ms = started.elapsed().as_millis() as u64;
        if souffleur_engine::stt::is_nonspeech(&out.text) {
            continue;
        }
        utt += 1;
        let ev = Event::TranscriptFinal {
            version: PROTOCOL_VERSION,
            t: t_capture_end,
            utterance_id: format!("w{utt}"),
            speaker: cfg.speaker.clone(),
            text: out.text,
            stt_latency_ms: Some(stt_ms),
        };
        emit(tx, cfg.print_stdout, &ev);
    }
    Ok(())
}

fn stream_mic(
    cfg: &Config,
    stt: &Stt,
    tx: &broadcast::Sender<String>,
    t0: Instant,
) -> Result<()> {
    let (rx, info) = audio::open_default_mic()?;
    eprintln!(
        "[mic] capturing: {} Hz, {} ch (default input device)",
        info.sample_rate, info.channels
    );
    let window_samples_dev = (cfg.window_ms as usize * info.sample_rate as usize) / 1000;
    let deadline = cfg.duration_s.map(|s| Instant::now() + Duration::from_secs(s));
    let mut buf: Vec<f32> = Vec::with_capacity(window_samples_dev * 2);
    let mut utt = 0u32;

    loop {
        if let Some(dl) = deadline {
            if Instant::now() >= dl {
                break;
            }
        }
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(chunk) => buf.extend_from_slice(&chunk),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
        if buf.len() < window_samples_dev {
            continue;
        }
        let window: Vec<f32> = buf.drain(..window_samples_dev).collect();
        let rms = (window.iter().map(|x| x * x).sum::<f32>() / window.len() as f32).sqrt();
        let audio16 = resample::resample_linear(&window, info.sample_rate, 16000);
        let t_capture_end = now_ms(t0);
        let started = Instant::now();
        let out = stt.transcribe(&audio16)?;
        let stt_ms = started.elapsed().as_millis() as u64;
        eprintln!(
            "[mic] window: rms={rms:.4} stt={stt_ms}ms text={:?}",
            out.text
        );
        if souffleur_engine::stt::is_nonspeech(&out.text) {
            continue;
        }
        utt += 1;
        let ev = Event::TranscriptFinal {
            version: PROTOCOL_VERSION,
            t: t_capture_end,
            utterance_id: format!("m{utt}"),
            speaker: cfg.speaker.clone(),
            text: out.text,
            stt_latency_ms: Some(stt_ms),
        };
        emit(tx, cfg.print_stdout, &ev);
    }
    Ok(())
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

    // Drain inbound (control messages) until the socket closes.
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
        None => return Ok(()), // --list-devices already printed
    };

    eprintln!("[core] loading model {} ...", cfg.model);
    let stt = Stt::load(&cfg.model, cfg.threads)?;
    let t0 = Instant::now();

    let (tx, _keepalive) = broadcast::channel::<String>(1024);
    let surfaces = Arc::new(AtomicUsize::new(0));

    // Capture + STT loop on a dedicated blocking thread.
    {
        let cfg = cfg.clone();
        let tx = tx.clone();
        let surfaces = surfaces.clone();
        std::thread::Builder::new()
            .name("souffleur-capture-loop".into())
            .spawn(move || {
                if let Err(e) = capture_loop(cfg, stt, tx, t0, surfaces) {
                    eprintln!("[capture] fatal: {e:#}");
                }
            })
            .context("spawn capture loop")?;
    }

    let listener = TcpListener::bind(&cfg.bind)
        .await
        .with_context(|| format!("bind {}", cfg.bind))?;
    eprintln!("[core] Coach Protocol WS listening on ws://{}", cfg.bind);

    loop {
        let (stream, _) = listener.accept().await.context("accept")?;
        let tx = tx.clone();
        let surfaces = surfaces.clone();
        tokio::spawn(handle_client(stream, tx, surfaces));
    }
}
