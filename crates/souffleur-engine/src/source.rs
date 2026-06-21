//! Audio sources, all normalized to 16 kHz mono f32 chunks on a channel.
//!
//! - [`spawn_mic`]: live default-input capture (the "me" channel).
//! - [`spawn_monitor`]: live system-audio loopback via PulseAudio `parec` on a
//!   `*.monitor` source (the "them" channel on this Linux dev box; the real
//!   targets on the user's machine are macOS Core Audio taps / Windows WASAPI).
//! - [`spawn_wav`]: a real WAV streamed in realtime-paced chunks (for tests and
//!   the file source).
//!
//! Every chunk is real captured/decoded audio — never synthesized.

use crate::{audio, resample};
use anyhow::{anyhow, Context, Result};
use crossbeam_channel::Receiver;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Duration;

const SR: u32 = 16_000;

/// Live microphone, resampled to 16 kHz mono. Returns the chunk receiver.
pub fn spawn_mic() -> Result<Receiver<Vec<f32>>> {
    let (dev_rx, info) = audio::open_default_mic()?;
    eprintln!(
        "[source:mic] {} Hz, {} ch (default input) -> 16k mono",
        info.sample_rate, info.channels
    );
    let (tx, rx) = crossbeam_channel::unbounded::<Vec<f32>>();
    std::thread::Builder::new()
        .name("souffleur-src-mic".into())
        .spawn(move || {
            // open_default_mic already downmixes to mono at the device rate.
            for chunk in dev_rx.iter() {
                let resampled = resample::resample_linear(&chunk, info.sample_rate, SR);
                if tx.send(resampled).is_err() {
                    break;
                }
            }
        })
        .context("spawn mic source")?;
    Ok(rx)
}

/// The default sink's monitor source name, e.g. `<sink>.monitor`.
pub fn default_monitor_name() -> Result<String> {
    let out = Command::new("pactl")
        .arg("get-default-sink")
        .output()
        .context("run pactl get-default-sink")?;
    if !out.status.success() {
        return Err(anyhow!("pactl get-default-sink failed"));
    }
    let sink = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if sink.is_empty() {
        return Err(anyhow!("no default sink"));
    }
    Ok(format!("{sink}.monitor"))
}

/// Live system-audio loopback via `parec` on a monitor source (16 kHz mono f32).
pub fn spawn_monitor(monitor: Option<String>) -> Result<Receiver<Vec<f32>>> {
    let monitor = match monitor {
        Some(m) => m,
        None => default_monitor_name()?,
    };
    eprintln!("[source:monitor] parec -d {monitor} (16k mono f32)");

    let mut child = Command::new("parec")
        .args([
            "--format=float32le",
            "--rate=16000",
            "--channels=1",
            "-d",
            &monitor,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn parec (is pulseaudio-utils installed?)")?;

    let mut stdout = child.stdout.take().ok_or_else(|| anyhow!("parec stdout"))?;
    let (tx, rx) = crossbeam_channel::unbounded::<Vec<f32>>();

    std::thread::Builder::new()
        .name("souffleur-src-monitor".into())
        .spawn(move || {
            // ~100 ms of audio per read at 16 kHz mono f32 = 1600 samples = 6400 bytes.
            let mut raw = [0u8; 6400];
            loop {
                match stdout.read(&mut raw) {
                    Ok(0) => break, // parec exited
                    Ok(n) => {
                        let n = n - (n % 4); // whole f32 samples only
                        let mut samples = Vec::with_capacity(n / 4);
                        for b in raw[..n].chunks_exact(4) {
                            samples.push(f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                        }
                        if tx.send(samples).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = child.kill();
        })
        .context("spawn monitor reader")?;
    Ok(rx)
}

/// Stream a real WAV in realtime-paced chunks at 16 kHz mono.
pub fn spawn_wav(path: &str, chunk_ms: u64) -> Result<Receiver<Vec<f32>>> {
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
    let audio16 = resample::resample_linear(&mono, spec.sample_rate, SR);
    let chunk = ((chunk_ms as usize) * SR as usize) / 1000;
    eprintln!(
        "[source:wav] {path}: {:.2}s -> 16k mono, {chunk_ms} ms chunks",
        audio16.len() as f64 / SR as f64
    );

    let (tx, rx) = crossbeam_channel::unbounded::<Vec<f32>>();
    std::thread::Builder::new()
        .name("souffleur-src-wav".into())
        .spawn(move || {
            for c in audio16.chunks(chunk.max(1)) {
                std::thread::sleep(Duration::from_secs_f64(c.len() as f64 / SR as f64));
                if tx.send(c.to_vec()).is_err() {
                    break;
                }
            }
        })
        .context("spawn wav source")?;
    Ok(rx)
}
