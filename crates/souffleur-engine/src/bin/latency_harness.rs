//! Souffleur latency harness.
//!
//! Feeds a REAL speech recording through the on-device STT path and reports real
//! transcription latency and realtime-factor. This is the Phase 0 make-or-break
//! measurement: it answers "can the local STT keep up with a live conversation?"
//! before any of the live pipeline is built on top of it.
//!
//! Usage: latency-harness [MODEL] [WAV] [N_THREADS] [RUNS]
//! Defaults: models/ggml-base.en.bin assets/jfk.wav 8 5

use anyhow::{Context, Result};
use souffleur_engine::{resample, stt::Stt};
use std::time::Instant;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let model = args.get(1).map(String::as_str).unwrap_or("models/ggml-base.en.bin");
    let wav = args.get(2).map(String::as_str).unwrap_or("assets/jfk.wav");
    let n_threads: i32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(8);
    let runs: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(5);

    // --- Load the real WAV ---
    let mut reader = hound::WavReader::open(wav).with_context(|| format!("open wav {wav}"))?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<std::result::Result<_, _>>()
            .context("reading float samples")?,
        hound::SampleFormat::Int => {
            let max = ((1i64 << (spec.bits_per_sample - 1)) - 1) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / max))
                .collect::<std::result::Result<_, _>>()
                .context("reading int samples")?
        }
    };
    let mono = resample::to_mono(&samples, spec.channels as usize);
    let audio16 = resample::resample_linear(&mono, spec.sample_rate, 16000);
    let audio_secs = audio16.len() as f64 / 16000.0;

    eprintln!("loading model {model} ...");
    let load_t = Instant::now();
    let stt = Stt::load(model, n_threads)?;
    let load_ms = load_t.elapsed().as_secs_f64() * 1000.0;
    eprintln!(
        "model loaded in {load_ms:.0} ms; audio = {audio_secs:.2}s (src {} Hz, {} ch -> 16k mono)",
        spec.sample_rate, spec.channels
    );

    // Warm-up (graph/buffer allocation) — not counted.
    let warm = stt.transcribe(&audio16)?;
    eprintln!("warm-up transcript: {}", warm.text);

    let mut times = Vec::with_capacity(runs);
    let mut text = String::new();
    for r in 0..runs {
        let t = Instant::now();
        let out = stt.transcribe(&audio16)?;
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        times.push(ms);
        text = out.text;
        eprintln!(
            "run {}/{}: {ms:.0} ms  (rtf {:.2}x)",
            r + 1,
            runs,
            (audio_secs * 1000.0) / ms
        );
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let min = times[0];
    let median = times[times.len() / 2];
    let max = *times.last().unwrap();
    let rtf = (audio_secs * 1000.0) / median;

    println!("\n=== Souffleur Phase-0 latency harness ===");
    println!("model:        {}", stt.model_label());
    println!("audio:        {wav}  ({audio_secs:.2}s, {} Hz src)", spec.sample_rate);
    println!("threads:      {n_threads}");
    println!("runs:         {runs}");
    println!("model load:   {load_ms:.0} ms (one-time)");
    println!("transcribe:   min {min:.0} ms | median {median:.0} ms | max {max:.0} ms (whole clip)");
    println!("realtime fac: {rtf:.2}x  (>1 = faster than real time = can keep up live)");
    println!("transcript:   {text}");
    println!(
        "\nNote: this is WHOLE-CLIP transcription latency. Streaming speech->text\n\
         lag (per rolling window) is a Phase-1 measurement; RTF here is the\n\
         viability signal that streaming can keep up."
    );
    Ok(())
}
