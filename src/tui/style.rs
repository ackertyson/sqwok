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
    (60, 190, 170),
    (230, 180, 80),
    (140, 200, 100),
    (200, 130, 230),
    (230, 130, 170),
    (230, 110, 80),
    (230, 160, 100),
    (160, 220, 180),
    (220, 200, 100),
    (180, 140, 200),
    (200, 180, 160),
];

// Closest 256-color xterm indices for each username color
const USERNAME_INDEXED: [u8; 12] = [
    167, // red-ish
    43,  // teal
    214, // orange-ish
    114, // green-ish
    177, // purple-ish
    211, // pink-ish
    173, // coral
    215, // orange
    115, // sea-green
    185, // yellow-ish
    140, // lavender
    181, // tan
];

pub fn username_color_by_index(join_idx: usize) -> Color {
    let idx = join_idx % USERNAME_RGB.len();
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

/// How many gradient cells to paint at the leading edge of the selection bar.
/// Truecolor gets 8 for a silky smooth ramp; 256-colour gets 4 since we're
/// limited to hand-picked indexed stops anyway.
pub fn selection_fade_steps() -> u16 {
    if is_truecolor() { 8 } else { 4 }
}

/// Interpolated color for the fade zone at the leading edge of the trailing
/// purple bar. Blends from `selection_bg` (text area) into `selection_trail_bg`
/// (the vivid bar) so the bar eases in rather than appearing with a hard edge.
/// `step` 0 is closest to the text; higher steps approach `selection_trail_bg`.
pub fn selection_bg_fade(step: u16, total_steps: u16) -> Color {
    let t = (step + 1) as f32 / (total_steps + 1) as f32;
    if is_truecolor() {
        let (r0, g0, b0) = (30u8, 30u8, 50u8);   // selection_bg
        let (r1, g1, b1) = (90u8, 60u8, 160u8);  // selection_trail_bg
        Color::Rgb(
            (r0 as f32 + (r1 - r0) as f32 * t) as u8,
            (g0 as f32 + (g1 - g0) as f32 * t) as u8,
            (b0 as f32 + (b1 - b0) as f32 * t) as u8,
        )
    } else {
        // Grayscale ramp (232-255) from near-black toward the light-gray trail.
        // Evenly spaced indices: 234→236→238→241→244→247.
        match step {
            0 => Color::Indexed(236),
            1 => Color::Indexed(238),
            2 => Color::Indexed(241),
            _ => Color::Indexed(244),
        }
    }
}
#[inline]
pub fn highlight_bg() -> Color {
    color((40, 40, 65), 236)
}
#[inline]
pub fn unread_bg() -> Color {
    color((45, 32, 0), 237)
}
#[inline]
pub fn error_color() -> Color {
    color((220, 80, 80), 167)
}
#[inline]
pub fn success_color() -> Color {
    color((80, 200, 120), 78)
}
#[inline]
pub fn shortcut_key_color() -> Color {
    color((180, 220, 140), 149)
}

#[inline]
pub fn selection_trail_bg() -> Color {
    color((90, 60, 160), 247)
}
#[inline]
pub fn warning_color() -> Color {
    color((230, 180, 80), 214)
}
#[inline]
pub fn typing_color() -> Color {
    color((230, 180, 80), 214)
}
#[inline]
pub fn overlay_bg() -> Color {
    color((15, 15, 25), 233)
}
#[inline]
pub fn modal_bg() -> Color {
    color((20, 20, 30), 234)
}
#[inline]
pub fn error_bg() -> Color {
    color((40, 15, 15), 52)
}

/// Style for selected vs unselected items in list views.
pub fn selected_style(is_selected: bool) -> ratatui::style::Style {
    use ratatui::style::{Modifier, Style};
    if is_selected {
        Style::default()
            .bg(selection_bg())
            .fg(fg())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(fg())
    }
}

/// Format a unix timestamp as a relative time string (e.g. "just now", "5m ago").
pub fn format_time_ago(unix_ts: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let diff = now - unix_ts;
    if diff < 60 {
        "just now".to_string()
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

/// Build a modal input line with label, current text, and cursor block.
pub fn input_line<'a>(label: &str, text: &str) -> ratatui::text::Line<'a> {
    use ratatui::{
        style::Style,
        text::{Line, Span},
    };
    Line::from(vec![
        Span::styled(label.to_string(), Style::default().fg(dim())),
        Span::styled(text.to_string(), Style::default().fg(fg())),
        Span::styled("█".to_string(), Style::default().fg(accent())),
    ])
}

// ── Hint bar helper ──────────────────────────────────────────────────────────

/// Build a colorized hint bar `Line` from a slice of (key, label) pairs.
/// The key symbol is rendered in `shortcut_key_color`, the label in `dim`.
pub fn hint_line(hints: &[(&str, &str)]) -> ratatui::text::Line<'static> {
    use ratatui::{
        style::Style,
        text::{Line, Span},
    };
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, (key, label)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", Style::default()));
        }
        spans.push(Span::styled(
            key.to_string(),
            Style::default().fg(shortcut_key_color()),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(label.to_string(), Style::default().fg(dim())));
    }
    Line::from(spans)
}
