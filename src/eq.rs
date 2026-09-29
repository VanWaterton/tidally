//! Equalizer presets, applied as an ffmpeg filter chain inside mpv.

/// Center frequencies of the 10 bands, in Hz.
pub const FREQS: [u32; 10] = [32, 64, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];

pub struct Preset {
    pub name: &'static str,
    /// Gain per band in dB.
    pub gains: [f32; 10],
}

pub const PRESETS: &[Preset] = &[
    Preset {
        name: "Flat",
        gains: [0.0; 10],
    },
    Preset {
        name: "Bass Boost",
        gains: [6.0, 5.0, 3.5, 1.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    },
    Preset {
        name: "Loudness",
        gains: [4.5, 3.5, 1.5, 0.0, -1.0, -1.0, 0.0, 1.5, 3.0, 4.0],
    },
    Preset {
        name: "Rock",
        gains: [4.0, 3.0, 1.5, -0.5, -1.0, 0.0, 1.5, 2.5, 3.0, 3.0],
    },
    Preset {
        name: "Electronic",
        gains: [5.0, 4.0, 1.0, 0.0, -1.5, 0.0, 1.0, 2.0, 3.5, 4.0],
    },
    Preset {
        name: "Hip-Hop",
        gains: [5.0, 4.5, 2.0, 1.0, -0.5, -0.5, 0.5, 0.5, 1.5, 2.0],
    },
    Preset {
        name: "Vocal",
        gains: [-2.0, -1.5, -0.5, 1.0, 2.5, 3.0, 2.5, 1.5, 0.0, -1.0],
    },
    Preset {
        name: "Acoustic",
        gains: [3.0, 2.5, 1.5, 0.5, 1.0, 1.0, 2.0, 2.5, 2.0, 1.5],
    },
    Preset {
        name: "Treble Boost",
        gains: [0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 2.0, 3.5, 5.0, 6.0],
    },
];

pub fn find(name: &str) -> usize {
    PRESETS
        .iter()
        .position(|p| p.name.eq_ignore_ascii_case(name))
        .unwrap_or(0)
}

impl Preset {
    /// The mpv `af` value for this preset, or `None` for flat (no filtering).
    pub fn filter(&self) -> Option<String> {
        let bands: Vec<String> = FREQS
            .iter()
            .zip(self.gains)
            .filter(|(_, g)| *g != 0.0)
            .map(|(f, g)| format!("equalizer=f={f}:t=o:w=1:g={g}"))
            .collect();
        if bands.is_empty() {
            return None;
        }
        // Pull the level down by the largest boost so the peaks don't clip.
        let max_boost = self.gains.iter().copied().fold(0.0, f32::max);
        Some(format!(
            "lavfi=[volume={:.1}dB,{}]",
            -max_boost,
            bands.join(",")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_has_no_filter() {
        assert!(PRESETS[0].filter().is_none());
    }

    #[test]
    fn filter_includes_preamp_and_bands() {
        let f = PRESETS[find("bass boost")].filter().unwrap();
        assert!(
            f.starts_with("lavfi=[volume=-6.0dB,equalizer=f=32:t=o:w=1:g=6"),
            "{f}"
        );
        assert!(!f.contains("f=1000"), "zero-gain bands are skipped: {f}");
    }
}
