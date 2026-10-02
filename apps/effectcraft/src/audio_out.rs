//! Audio preview output through cpal (the default output device).
//!
//! cpal streams are not `Send` on every platform, so each preview runs its stream on a small
//! dedicated thread that owns it until stopped. The callback pulls interleaved stereo from the
//! frontend's [`AudioFeed`] and maps it onto the device's channels (mono: L+R average; extra
//! channels: silence), converting to the device's sample format.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use effectcraft_ui_egui::audio::{AudioDevice, AudioFeed};

pub struct CpalOut {
    rate: u32,
    latency: Arc<AtomicU64>,
    stop: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

/// Open the default output device (`None` without one).
pub fn open() -> Option<Box<dyn AudioDevice>> {
    let host = cpal::default_host();
    let dev = host.default_output_device()?;
    let cfg = dev.default_output_config().ok()?;
    Some(Box::new(CpalOut { rate: cfg.sample_rate().0, latency: Arc::new(AtomicU64::new(0)), stop: None, thread: None }))
}

fn run<T: SizedSample + FromSample<f32>>(
    dev: &cpal::Device,
    cfg: &cpal::StreamConfig,
    feed: Arc<AudioFeed>,
    latency: Arc<AtomicU64>,
) -> Result<cpal::Stream, String> {
    let ch = cfg.channels as usize;
    let rate = cfg.sample_rate.0 as f64;
    let mut stereo: Vec<f32> = Vec::new();
    dev.build_output_stream(
        cfg,
        move |out: &mut [T], info: &cpal::OutputCallbackInfo| {
            let frames = out.len() / ch.max(1);
            stereo.resize(frames * 2, 0.0);
            feed.pull(&mut stereo);
            for (f, s) in out.chunks_mut(ch.max(1)).zip(stereo.chunks_exact(2)) {
                for (c, o) in f.iter_mut().enumerate() {
                    let v = match (ch, c) {
                        (1, _) => (s[0] + s[1]) * 0.5,
                        (_, 0) => s[0],
                        (_, 1) => s[1],
                        _ => 0.0,
                    };
                    *o = T::from_sample(v.clamp(-1.0, 1.0));
                }
            }
            // Frames between "handed over" and "audible": device latency plus this buffer.
            let ts = info.timestamp();
            let dev_lat = ts.playback.duration_since(&ts.callback).map(|d| d.as_secs_f64()).unwrap_or(0.0);
            latency.store((dev_lat * rate) as u64 + frames as u64, Ordering::Relaxed);
        },
        |e| eprintln!("effectcraft: audio output: {e}"),
        None,
    )
    .map_err(|e| e.to_string())
}

impl AudioDevice for CpalOut {
    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn start(&mut self, feed: Arc<AudioFeed>) -> Result<(), String> {
        self.stop();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let latency = self.latency.clone();
        let rate = self.rate;
        let thread = std::thread::Builder::new()
            .name("ec-audio-out".into())
            .spawn(move || {
                let host = cpal::default_host();
                let Some(dev) = host.default_output_device() else {
                    let _ = ready_tx.send(Err("no audio output device".into()));
                    return;
                };
                let sup = match dev.default_output_config() {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e.to_string()));
                        return;
                    }
                };
                let mut cfg: cpal::StreamConfig = sup.config();
                cfg.sample_rate = cpal::SampleRate(rate);
                let stream = match sup.sample_format() {
                    cpal::SampleFormat::F32 => run::<f32>(&dev, &cfg, feed, latency),
                    cpal::SampleFormat::I16 => run::<i16>(&dev, &cfg, feed, latency),
                    cpal::SampleFormat::U16 => run::<u16>(&dev, &cfg, feed, latency),
                    cpal::SampleFormat::I32 => run::<i32>(&dev, &cfg, feed, latency),
                    f => Err(format!("unsupported sample format {f:?}")),
                };
                let stream = match stream.and_then(|s| s.play().map(|_| s).map_err(|e| e.to_string())) {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                let _ = stop_rx.recv();
                drop(stream);
            })
            .map_err(|e| e.to_string())?;
        self.stop = Some(stop_tx);
        self.thread = Some(thread);
        ready_rx.recv().map_err(|e| e.to_string())?
    }

    fn stop(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    fn latency_frames(&self) -> u64 {
        self.latency.load(Ordering::Relaxed)
    }
}

impl Drop for CpalOut {
    fn drop(&mut self) {
        self.stop();
    }
}
