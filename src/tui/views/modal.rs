use ratatui::{
    layout::{Alignment, Rect},
    style::Style,
    widgets::{Block, Borders, Clear},
    Frame,
};

use crate::tui::style as s;

pub fn draw_modal_frame(frame: &mut Frame, title: &str, content_fn: impl FnOnce(&mut Frame, Rect)) {
    let area = frame.area();

    let width = ((area.width as f32 * 0.6) as u16).max(40).min(area.width);
    let height = ((area.height as f32 * 0.7) as u16).max(15).min(area.height);
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, modal_area);

    let block = Block::default()
        .title(format!(" {} ", title))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(s::accent()))
        .style(Style::default().bg(s::modal_bg()));

    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    content_fn(frame, inner);
}
