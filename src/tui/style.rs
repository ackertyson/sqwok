use ratatui::style::Color;
use std::sync::OnceLock;

/// Detect if the terminal supports truecolor (24-bit RGB).
fn supports_truecolor() -> bool {
    match std::env::var("COLORTERM") {
        Ok(val) => matches!(val.as_str(), "truecolor" | "24bit"),
        Err(_) => false,
    }
}

fn is_truecolor() -> bool {
    static TRUECOLOR: OnceLock<bool> = OnceLock::new();
    *TRUECOLOR.get_or_init(supports_truecolor)
}

/// Select RGB if truecolor is supported, otherwise use 256-color indexed fallback.
fn color(rgb: (u8, u8, u8), indexed: u8) -> Color {
    if is_truecolor() {
        Color::Rgb(rgb.0, rgb.1, rgb.2)
    } else {
        Color::Indexed(indexed)
    }
}

// ── Username palette ────────────────────────────────────────────────────────

const USERNAME_RGB: [(u8, u8, u8); 12] = [
    (230, 100, 100),
    (100, 200, 230),
    (230, 180, 80),
    (140, 200, 100),
    (200, 130, 230),
    (230, 130, 170),
    (100, 180, 230),
    (230, 160, 100),
    (160, 220, 180),
    (220, 200, 100),
    (180, 140, 200),
    (200, 180, 160),
];

// Closest 256-color xterm indices for each username color
const USERNAME_INDEXED: [u8; 12] = [
    167, // red-ish
    81,  // blue-ish
    214, // orange-ish
    114, // green-ish
    177, // purple-ish
    211, // pink-ish
    75,  // steel-blue
    215, // orange
    115, // sea-green
    185, // yellow-ish
    140, // lavender
    181, // tan
];

pub fn username_color(uuid_str: &str) -> Color {
    let first_byte = uuid_str.as_bytes().first().copied().unwrap_or(0) as usize;
    let idx = first_byte % USERNAME_RGB.len();
    let (r, g, b) = USERNAME_RGB[idx];
    color((r, g, b), USERNAME_INDEXED[idx])
}

// ── Theme colors ────────────────────────────────────────────────────────────
// These were previously `pub const` — now they're inline functions so that
// 256-color fallback is transparent to callers (same `s::ACCENT` syntax).

pub const BG: Color = Color::Reset;

#[inline]
pub fn fg() -> Color {
    color((200, 200, 210), 252)
}
#[inline]
pub fn dim() -> Color {
    color((90, 90, 110), 60)
}
#[inline]
pub fn accent() -> Color {
    color((100, 180, 255), 75)
}
#[inline]
pub fn selection_bg() -> Color {
    color((30, 30, 50), 234)
}
#[inline]
pub fn highlight_bg() -> Color {
    color((40, 40, 65), 236)
}
#[inline]
pub fn error_color() -> Color {
    color((220, 80, 80), 167)
}
#[inline]
pub fn success_color() -> Color {
    color((80, 200, 120), 78)
}
