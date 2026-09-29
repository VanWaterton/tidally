//! Album cover fetching, palette extraction, and terminal-graphics encoding.

use std::sync::Arc;

use anyhow::Result;
use image::DynamicImage;
use ratatui::layout::Size;
use ratatui_image::Resize;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use tokio::sync::mpsc::UnboundedSender;

use crate::event::AppEvent;

pub type Rgb = (u8, u8, u8);

pub struct Palette {
    pub accent: Rgb,
    pub secondary: Rgb,
}

/// Cover-art state for the current track.
pub struct Art {
    picker: Picker,
    /// Cover id we want displayed (from the current track's album).
    pub cover: Option<String>,
    image: Option<Arc<DynamicImage>>,
    /// Encoded protocol for one cell size; re-encoded when the area changes.
    pub protocol: Option<(Size, Protocol)>,
    pending: Option<Size>,
    /// Largest area the UI wanted to draw the cover in during the last frame.
    pub wanted: Option<Size>,
}

impl Art {
    pub fn new(picker: Picker) -> Self {
        Self {
            picker,
            cover: None,
            image: None,
            protocol: None,
            pending: None,
            wanted: None,
        }
    }

    /// Terminal cell aspect: pixels tall per pixel wide, for sizing square covers.
    pub fn cell_aspect(&self) -> f32 {
        let fs = self.picker.font_size();
        if fs.width == 0 {
            2.0
        } else {
            fs.height as f32 / fs.width as f32
        }
    }

    /// Switches to a new cover and starts downloading it. No-op if it's already current.
    pub fn show(
        &mut self,
        cover: Option<String>,
        http: &reqwest::Client,
        tx: &UnboundedSender<AppEvent>,
    ) {
        if cover == self.cover {
            return;
        }
        self.cover = cover.clone();
        self.image = None;
        self.protocol = None;
        self.pending = None;
        let Some(cover) = cover else { return };
        let (http, tx) = (http.clone(), tx.clone());
        tokio::spawn(async move {
            if let Ok((image, palette)) = fetch(&http, &cover).await {
                let _ = tx.send(AppEvent::CoverLoaded {
                    cover,
                    image,
                    palette,
                });
            }
        });
    }

    pub fn on_loaded(&mut self, cover: String, image: Arc<DynamicImage>) -> bool {
        if self.cover.as_deref() != Some(&cover) {
            return false;
        }
        self.image = Some(image);
        true
    }

    pub fn on_encoded(&mut self, cover: String, size: Size, protocol: Protocol) -> bool {
        if self.cover.as_deref() != Some(&cover) {
            return false;
        }
        self.pending = None;
        self.protocol = Some((size, protocol));
        true
    }

    /// Kicks off a background encode if the UI wants the cover at a size we don't have yet.
    /// Encoding (especially sixel) is slow, so it never happens on the render path.
    pub fn sync(&mut self, tx: &UnboundedSender<AppEvent>) {
        let (Some(want), Some(image), Some(cover)) = (self.wanted, &self.image, &self.cover) else {
            return;
        };
        let have = self.protocol.as_ref().map(|(s, _)| *s);
        if have == Some(want) || self.pending == Some(want) || want.width == 0 || want.height == 0 {
            return;
        }
        self.pending = Some(want);
        let (picker, image, cover, tx) = (
            self.picker.clone(),
            image.clone(),
            cover.clone(),
            tx.clone(),
        );
        tokio::task::spawn_blocking(move || {
            let filter = Resize::Fit(Some(image::imageops::FilterType::Triangle));
            if let Ok(protocol) = picker.new_protocol((*image).clone(), want, filter) {
                let _ = tx.send(AppEvent::CoverEncoded {
                    cover,
                    size: want,
                    protocol,
                });
            }
        });
    }
}

fn cover_url(cover: &str, px: u32) -> String {
    format!(
        "https://resources.tidal.com/images/{}/{px}x{px}.jpg",
        cover.replace('-', "/")
    )
}

async fn fetch(http: &reqwest::Client, cover: &str) -> Result<(Arc<DynamicImage>, Palette)> {
    let bytes = http
        .get(cover_url(cover, 640))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    tokio::task::spawn_blocking(move || {
        let image = image::load_from_memory(&bytes)?;
        let palette = extract_palette(&image);
        Ok((Arc::new(image), palette))
    })
    .await?
}

/// Picks the two most prominent vivid hues from the cover, brightened for a dark background.
pub fn extract_palette(image: &DynamicImage) -> Palette {
    const BINS: usize = 18;
    let thumb = image.thumbnail(32, 32).to_rgb8();
    let mut weight = [0f32; BINS];
    let mut sums = [(0f32, 0f32, 0f32); BINS];
    for p in thumb.pixels() {
        let (h, s, v) = rgb_to_hsv(p[0], p[1], p[2]);
        // Favor saturated, reasonably bright pixels; ignore near-greys.
        let w = s * s * v;
        if s < 0.18 || v < 0.15 {
            continue;
        }
        let bin = ((h / 360.0) * BINS as f32) as usize % BINS;
        weight[bin] += w;
        sums[bin].0 += h * w;
        sums[bin].1 += s * w;
        sums[bin].2 += v * w;
    }

    let mut order: Vec<usize> = (0..BINS).collect();
    order.sort_by(|a, b| weight[*b].total_cmp(&weight[*a]));
    let total: f32 = weight.iter().sum();
    let hue_of = |bin: usize| {
        let w = weight[bin];
        (sums[bin].0 / w, sums[bin].1 / w, sums[bin].2 / w)
    };

    if total < 1.0 {
        // Greyscale cover: keep the default palette.
        return Palette {
            accent: crate::ui::DEFAULT_ACCENT,
            secondary: crate::ui::DEFAULT_SECONDARY,
        };
    }
    let (h1, s1, v1) = hue_of(order[0]);
    // Second color: the next strong bin that's a visibly different hue, else rotate the first.
    let second = order[1..]
        .iter()
        .copied()
        .find(|&b| weight[b] > total * 0.08 && hue_distance(hue_of(b).0, h1) > 35.0);
    let (h2, s2, v2) = match second {
        Some(b) => hue_of(b),
        None => ((h1 + 55.0) % 360.0, s1, v1),
    };
    Palette {
        accent: vivid(h1, s1, v1),
        secondary: vivid(h2, s2, v2),
    }
}

/// Clamp saturation and brightness so the color reads well as text on a dark background.
fn vivid(h: f32, s: f32, v: f32) -> Rgb {
    hsv_to_rgb(h, s.clamp(0.45, 0.85), v.max(0.88))
}

fn hue_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).abs() % 360.0;
    d.min(360.0 - d)
}

fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max == 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Rgb {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to = |v: f32| ((v + m) * 255.0).round() as u8;
    (to(r), to(g), to(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb as Px, RgbImage};

    #[test]
    fn hsv_round_trip() {
        for rgb in [(255, 0, 0), (0, 200, 100), (30, 60, 220), (250, 180, 20)] {
            let (h, s, v) = rgb_to_hsv(rgb.0, rgb.1, rgb.2);
            let back = hsv_to_rgb(h, s, v);
            assert!(
                (back.0 as i32 - rgb.0 as i32).abs() <= 1,
                "{rgb:?} -> {back:?}"
            );
            assert!(
                (back.1 as i32 - rgb.1 as i32).abs() <= 1,
                "{rgb:?} -> {back:?}"
            );
            assert!(
                (back.2 as i32 - rgb.2 as i32).abs() <= 1,
                "{rgb:?} -> {back:?}"
            );
        }
    }

    #[test]
    fn palette_finds_dominant_hues() {
        // Left 70% red, right 30% blue.
        let img = RgbImage::from_fn(100, 100, |x, _| {
            if x < 70 {
                Px([220, 30, 30])
            } else {
                Px([30, 60, 220])
            }
        });
        let p = extract_palette(&DynamicImage::ImageRgb8(img));
        assert!(
            p.accent.0 > p.accent.2,
            "accent should be red: {:?}",
            p.accent
        );
        assert!(
            p.secondary.2 > p.secondary.0,
            "secondary should be blue: {:?}",
            p.secondary
        );
    }

    #[test]
    fn greyscale_keeps_default() {
        let img = RgbImage::from_fn(10, 10, |x, _| Px([x as u8 * 20; 3]));
        let p = extract_palette(&DynamicImage::ImageRgb8(img));
        assert_eq!(p.accent, crate::ui::DEFAULT_ACCENT);
    }
}
