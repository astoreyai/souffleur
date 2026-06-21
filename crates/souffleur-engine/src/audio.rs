//! Live audio capture via `cpal`.
//!
//! Phase 0 captures the local microphone (the "me" channel) from a real input
//! device. The system-audio loopback (the "them" channel) is the real target on
//! the user's meeting machine via Core Audio process taps (macOS) / WASAPI
//! loopback (Windows); on this Linux dev box it is a PulseAudio/PipeWire monitor
//! source, wired in Phase 1. Capture here is real PCM from a real device — never
//! synthesized.

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};

/// Metadata about a started capture stream.
#[derive(Debug, Clone, Copy)]
pub struct CaptureInfo {
    pub sample_rate: u32,
    pub channels: u16,
}

/// Names of all available input devices on the default host.
pub fn list_input_devices() -> Result<Vec<String>> {
    let host = cpal::default_host();
    let mut out = Vec::new();
    let default = host
        .default_input_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_default();
    for dev in host.input_devices().context("enumerate input devices")? {
        let name = dev.name().unwrap_or_else(|_| "<unknown>".into());
        let cfg = dev
            .default_input_config()
            .map(|c| format!("{} Hz, {} ch, {:?}", c.sample_rate().0, c.channels(), c.sample_format()))
            .unwrap_or_else(|_| "<no default config>".into());
        let marker = if name == default { " (default)" } else { "" };
        out.push(format!("{name}{marker}  [{cfg}]"));
    }
    Ok(out)
}

/// Start capturing from the default input device on a dedicated thread.
///
/// Interleaved frames are downmixed to mono and pushed to `tx` as `Vec<f32>`
/// chunks (one per cpal callback). The cpal stream lives on the spawned thread
/// and is kept alive until the process exits (cpal streams are not `Send` on all
/// hosts, so we never move it across threads). Returns the device's real sample
/// rate / channel count.
pub fn spawn_default_mic_capture(tx: Sender<Vec<f32>>) -> Result<CaptureInfo> {
    let (init_tx, init_rx) = crossbeam_channel::bounded::<Result<CaptureInfo>>(1);

    std::thread::Builder::new()
        .name("souffleur-capture".into())
        .spawn(move || {
            let built = build_and_play(tx);
            match built {
                Ok((info, stream)) => {
                    // Hold the stream for the process lifetime (cpal's callback runs
                    // on its own audio thread; dropping this would stop capture).
                    let _stream = stream;
                    let _ = init_tx.send(Ok(info));
                    loop {
                        std::thread::park();
                    }
                }
                Err(e) => {
                    let _ = init_tx.send(Err(e));
                }
            }
        })
        .context("spawn capture thread")?;

    init_rx
        .recv()
        .context("capture thread init channel closed")?
}

fn build_and_play(tx: Sender<Vec<f32>>) -> Result<(CaptureInfo, cpal::Stream)> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("no default input device"))?;
    let supported = device
        .default_input_config()
        .context("default_input_config")?;
    let info = CaptureInfo {
        sample_rate: supported.sample_rate().0,
        channels: supported.channels(),
    };
    let channels = info.channels as usize;
    let err_fn = |e| eprintln!("[capture] stream error: {e}");
    let config: cpal::StreamConfig = supported.clone().into();

    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                tx.try_send(downmix_mono(data, channels)).ok();
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                let f: Vec<f32> = data.iter().map(|&s| s as f32 / 32768.0).collect();
                tx.try_send(downmix_mono(&f, channels)).ok();
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::U16 => device.build_input_stream(
            &config,
            move |data: &[u16], _: &cpal::InputCallbackInfo| {
                let f: Vec<f32> = data.iter().map(|&s| (s as f32 - 32768.0) / 32768.0).collect();
                tx.try_send(downmix_mono(&f, channels)).ok();
            },
            err_fn,
            None,
        )?,
        other => return Err(anyhow!("unsupported sample format: {other:?}")),
    };
    stream.play().context("stream.play")?;
    Ok((info, stream))
}

fn downmix_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks(channels)
        .map(|frame| frame.iter().copied().sum::<f32>() / channels as f32)
        .collect()
}

/// Convenience: a receiver paired to a freshly started default-mic capture.
pub fn open_default_mic() -> Result<(Receiver<Vec<f32>>, CaptureInfo)> {
    let (tx, rx) = crossbeam_channel::unbounded::<Vec<f32>>();
    let info = spawn_default_mic_capture(tx)?;
    Ok((rx, info))
}
