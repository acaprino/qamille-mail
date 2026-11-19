//! Meeting recorder: captures the default microphone into a WAV file, with pause/resume.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

type Writer = hound::WavWriter<std::io::BufWriter<std::fs::File>>;

pub struct Recorder {
    stream: cpal::Stream,
    writer: Arc<Mutex<Option<Writer>>>,
    paused: Arc<AtomicBool>,
    frames: Arc<AtomicU64>,
    level: Arc<AtomicU64>,
    pub sample_rate: u32,
    pub path: PathBuf,
}

impl Recorder {
    pub fn start(path: PathBuf) -> Result<Recorder, String> {
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or("No microphone found.")?;
        let config = device.default_input_config().map_err(|e| e.to_string())?;
        let channels = config.channels();
        let sample_rate = config.sample_rate().0;
        let spec = hound::WavSpec { channels: 1, sample_rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let writer = Arc::new(Mutex::new(Some(hound::WavWriter::create(&path, spec).map_err(|e| e.to_string())?)));
        let paused = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(AtomicU64::new(0));
        let level = Arc::new(AtomicU64::new(0));

        fn push(samples: impl Iterator<Item = f32>, ch: u16, w: &Arc<Mutex<Option<Writer>>>, paused: &AtomicBool, frames: &AtomicU64, level: &AtomicU64) {
            if paused.load(Ordering::Relaxed) {
                return;
            }
            let mut g = w.lock().unwrap();
            let Some(wr) = g.as_mut() else { return };
            let mut acc = 0.0f32;
            let mut n = 0u32;
            let mut peak = 0.0f32;
            for s in samples {
                acc += s;
                n += 1;
                if n == ch as u32 {
                    let v = (acc / ch as f32).clamp(-1.0, 1.0);
                    peak = peak.max(v.abs());
                    let _ = wr.write_sample((v * i16::MAX as f32) as i16);
                    frames.fetch_add(1, Ordering::Relaxed);
                    acc = 0.0;
                    n = 0;
                }
            }
            level.store((peak * 1000.0) as u64, Ordering::Relaxed);
        }

        let err = |e| eprintln!("recorder: {e}");
        let (w, p, f, l) = (writer.clone(), paused.clone(), frames.clone(), level.clone());
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config.clone().into(),
                move |d: &[f32], _: &_| push(d.iter().copied(), channels, &w, &p, &f, &l),
                err,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config.clone().into(),
                move |d: &[i16], _: &_| push(d.iter().map(|s| *s as f32 / i16::MAX as f32), channels, &w, &p, &f, &l),
                err,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_input_stream(
                &config.clone().into(),
                move |d: &[u16], _: &_| push(d.iter().map(|s| (*s as f32 - 32768.0) / 32768.0), channels, &w, &p, &f, &l),
                err,
                None,
            ),
            other => return Err(format!("Unsupported microphone format {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Recorder { stream, writer, paused, frames, level, sample_rate, path })
    }

    pub fn set_paused(&self, p: bool) {
        self.paused.store(p, Ordering::Relaxed);
    }
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }
    pub fn seconds(&self) -> u64 {
        self.frames.load(Ordering::Relaxed) / self.sample_rate.max(1) as u64
    }
    pub fn level(&self) -> f32 {
        self.level.load(Ordering::Relaxed) as f32 / 1000.0
    }
    pub fn finish(self) -> Result<(PathBuf, u64), String> {
        let secs = self.seconds();
        let _ = self.stream.pause();
        drop(self.stream);
        if let Some(w) = self.writer.lock().unwrap().take() {
            w.finalize().map_err(|e| e.to_string())?;
        }
        Ok((self.path, secs))
    }
}
