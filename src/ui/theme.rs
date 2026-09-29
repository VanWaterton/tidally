//! Colors. The accent pair is set at runtime from the current album cover, so it's stored in
//! atomics rather than constants; everything else is fixed.

use std::sync::atomic::{AtomicU32, Ordering};

use ratatui::style::{Color, Modifier, Style};

pub const BASE: Color = Color::Rgb(16, 17, 24);
pub const TEXT: Color = Color::Rgb(230, 232, 242);
pub const MUTED: Color = Color::Rgb(140, 144, 166);
pub const FAINT: Color = Color::Rgb(78, 82, 102);
pub const BORDER: Color = Color::Rgb(50, 54, 72);
pub const SUCCESS: Color = Color::Rgb(120, 226, 160);
pub const ERROR: Color = Color::Rgb(255, 110, 120);
pub const BADGE: Color = Color::Rgb(250, 204, 21);

pub const DEFAULT_ACCENT: (u8, u8, u8) = (64, 212, 255);
pub const DEFAULT_SECONDARY: (u8, u8, u8) = (178, 110, 255);

static ACCENT: AtomicU32 = AtomicU32::new(pack(DEFAULT_ACCENT));
static SECONDARY: AtomicU32 = AtomicU32::new(pack(DEFAULT_SECONDARY));

const fn pack((r, g, b): (u8, u8, u8)) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

fn unpack(v: u32) -> Color {
    Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub fn set_palette(accent: (u8, u8, u8), secondary: (u8, u8, u8)) {
    ACCENT.store(pack(accent), Ordering::Relaxed);
    SECONDARY.store(pack(secondary), Ordering::Relaxed);
}

pub fn accent_color() -> Color {
    unpack(ACCENT.load(Ordering::Relaxed))
}

pub fn secondary_color() -> Color {
    unpack(SECONDARY.load(Ordering::Relaxed))
}

pub fn accent_dim() -> Color {
    lerp(BASE, accent_color(), 0.5)
}

/// Row highlight: the background tinted towards the accent.
pub fn selection_bg() -> Color {
    lerp(BASE, accent_color(), 0.22)
}

/// Linear blend between two RGB colors; `t` in 0..=1.
pub fn lerp(a: Color, b: Color, t: f32) -> Color {
    let (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) = (a, b) else {
        return b;
    };
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)).round() as u8;
    Color::Rgb(mix(ar, br), mix(ag, bg), mix(ab, bb))
}

/// A point along the accent → secondary gradient.
pub fn gradient(t: f32) -> Color {
    lerp(accent_color(), secondary_color(), t)
}

pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn spinner(frame: usize) -> &'static str {
    SPINNER[(frame / 3) % SPINNER.len()]
}

pub fn text() -> Style {
    Style::new().fg(TEXT)
}

pub fn muted() -> Style {
    Style::new().fg(MUTED)
}

pub fn faint() -> Style {
    Style::new().fg(FAINT)
}

pub fn accent() -> Style {
    Style::new().fg(accent_color())
}

pub fn secondary() -> Style {
    Style::new().fg(secondary_color())
}

pub fn bold_accent() -> Style {
    accent().add_modifier(Modifier::BOLD)
}

pub fn border(focused: bool) -> Style {
    Style::new().fg(if focused { accent_dim() } else { BORDER })
}

pub fn selected_row() -> Style {
    Style::new().bg(selection_bg()).add_modifier(Modifier::BOLD)
}
