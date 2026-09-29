//! Spectrum analyzer fed by the PipeWire/PulseAudio monitor of the default output.
//!
//! mpv doesn't expose decoded PCM over IPC, so like cava we capture what's going to the speakers
//! (via `parec`) and run an FFT on it in a background thread.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

const RATE: usize = 44_100;
const FFT_SIZE: usize = 4096;
/// New analysis every ~16ms of audio (60 updates/s).
const HOP: usize = RATE / 60;
pub const BANDS: usize = 64;
const MIN_HZ: f32 = 35.0;
const MAX_HZ: f32 = 16_000.0;

#[derive(Default)]
struct Shared {
    bands: Vec<f32>,
    error: Option<String>,
}

pub struct Visualizer {
    shared: Arc<Mutex<Shared>>,
    child: Option<Child>,
}

impl Visualizer {
    /// Captures from the sound server mpv plays to, so a forwarded SSH server is visualized too.
    pub fn start(pulse_server: Option<&str>) -> Self {
        let shared = Arc::new(Mutex::new(Shared {
            bands: vec![0.0; BANDS],
            error: None,
        }));
        let mut cmd = Command::new("parec");
        #[cfg(feature = "remote")]
        if let Some(server) = pulse_server {
            cmd.env("PULSE_SERVER", server);
        }
        #[cfg(not(feature = "remote"))]
        let _ = pulse_server;
        let spawned = cmd
            .args([
                "--device=@DEFAULT_MONITOR@",
                "--format=float32le",
                "--channels=1",
                "--raw",
                "--latency-msec=15",
                "--client-name=tidally-visualizer",
            ])
            .arg(format!("--rate={RATE}"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();

        let mut child = match spawned {
            Ok(child) => child,
            Err(e) => {
                shared.lock().unwrap().error = Some(format!("parec unavailable: {e}"));
                return Self {
                    shared,
                    child: None,
                };
            }
        };
        let stdout = child.stdout.take().expect("stdout is piped");
        let worker = shared.clone();
        thread::Builder::new()
            .name("visualizer".into())
            .spawn(move || analyze(stdout, worker))
            .expect("spawn visualizer thread");
        Self {
            shared,
            child: Some(child),
        }
    }

    /// Current band levels, each in 0..=1.
    pub fn bands(&self) -> Vec<f32> {
        self.shared.lock().unwrap().bands.clone()
    }

    pub fn error(&self) -> Option<String> {
        self.shared.lock().unwrap().error.clone()
    }
}

impl Drop for Visualizer {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn analyze(mut input: impl Read, shared: Arc<Mutex<Shared>>) {
    let fft = FftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
    let window: Vec<f32> = (0..FFT_SIZE)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / FFT_SIZE as f32).cos())
        .collect();
    let edges = band_edges();

    let mut ring = vec![0f32; FFT_SIZE];
    let mut ring_pos = 0;
    let mut bytes = [0u8; HOP * 4];
    let mut buf = vec![Complex::new(0.0, 0.0); FFT_SIZE];
    let mut levels = vec![0f32; BANDS];
    // Slowly-adapting peak for auto gain, so quiet and loud masters both fill the display.
    let mut peak = 1e-3f32;

    loop {
        if input.read_exact(&mut bytes).is_err() {
            shared.lock().unwrap().error = Some("audio capture stopped".into());
            return;
        }
        for chunk in bytes.as_chunks::<4>().0 {
            ring[ring_pos] = f32::from_le_bytes(*chunk);
            ring_pos = (ring_pos + 1) % FFT_SIZE;
        }
        for (i, c) in buf.iter_mut().enumerate() {
            *c = Complex::new(ring[(ring_pos + i) % FFT_SIZE] * window[i], 0.0);
        }
        fft.process(&mut buf);

        let mut raw = [0f32; BANDS];
        for (b, value) in raw.iter_mut().enumerate() {
            let (lo, hi) = edges[b];
            let energy = buf[lo..hi].iter().map(|c| c.norm_sqr()).fold(0.0, f32::max);
            // Tilt up the highs a little; music energy falls off roughly 3dB/octave.
            let tilt = 1.0 + b as f32 / BANDS as f32 * 2.5;
            *value = energy.sqrt() * tilt;
        }

        let frame_peak = raw.iter().copied().fold(0.0, f32::max);
        peak = if frame_peak > peak {
            frame_peak
        } else {
            (peak * 0.998).max(1e-3)
        };

        for (level, value) in levels.iter_mut().zip(raw) {
            // Perceptual compression, then fast attack / gravity-style fall.
            let target = (value / peak).clamp(0.0, 1.0).powf(0.6);
            *level = if target > *level {
                *level * 0.3 + target * 0.7
            } else {
                (*level - 0.035).max(target)
            };
        }
        shared.lock().unwrap().bands.copy_from_slice(&levels);
    }
}

/// Log-spaced FFT bin ranges for each band, each at least one bin wide.
fn band_edges() -> Vec<(usize, usize)> {
    let hz_per_bin = RATE as f32 / FFT_SIZE as f32;
    let ratio = (MAX_HZ / MIN_HZ).powf(1.0 / BANDS as f32);
    let mut edges = Vec::with_capacity(BANDS);
    let mut lo_bin = (MIN_HZ / hz_per_bin).floor() as usize;
    for b in 0..BANDS {
        let hi_hz = MIN_HZ * ratio.powi(b as i32 + 1);
        let hi_bin = ((hi_hz / hz_per_bin).ceil() as usize)
            .max(lo_bin + 1)
            .min(FFT_SIZE / 2);
        edges.push((lo_bin, hi_bin));
        lo_bin = hi_bin.min(FFT_SIZE / 2 - 1);
    }
    edges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_edges_are_monotonic_and_nonempty() {
        let edges = band_edges();
        assert_eq!(edges.len(), BANDS);
        for w in edges.windows(2) {
            assert!(w[0].1 <= w[1].1);
        }
        assert!(edges.iter().all(|(lo, hi)| hi > lo));
    }

    #[test]
    fn sine_lights_up_the_right_band() {
        // 1kHz tone, a bit over one FFT window's worth.
        let samples: Vec<u8> = (0..FFT_SIZE + HOP * 4)
            .flat_map(|i| {
                (0.5 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / RATE as f32).sin())
                    .to_le_bytes()
            })
            .collect();
        let shared = Arc::new(Mutex::new(Shared {
            bands: vec![0.0; BANDS],
            error: None,
        }));
        analyze(std::io::Cursor::new(samples), shared.clone());
        let bands = shared.lock().unwrap().bands.clone();
        let loudest = bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        let (lo, hi) = band_edges()[loudest];
        let hz_per_bin = RATE as f32 / FFT_SIZE as f32;
        assert!(
            lo as f32 * hz_per_bin <= 1100.0 && hi as f32 * hz_per_bin >= 900.0,
            "band {loudest}"
        );
    }
}
