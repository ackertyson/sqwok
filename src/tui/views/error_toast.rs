use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::{
    app::{AppState, ConnStatus},
    style as s,
};

pub fn draw(frame: &mut Frame, app: &AppState) {
    let area = frame.area();
    let width = (area.width / 3).clamp(36, 50);

    let lines: Vec<Line> = match &app.connection_status {
        ConnStatus::Disconnected { ref reason, since } => {
            let elapsed = since.elapsed().as_secs();
            // Truncate long reason to fit within the box
            let max_reason_len = (width as usize).saturating_sub(2);
            let reason_display: String = reason.chars().take(max_reason_len).collect();
            vec![
                Line::from(Span::styled(
                    "Disconnected",
                    Style::default().fg(s::error_color()),
                )),
                Line::from(Span::styled(reason_display, Style::default().fg(s::dim()))),
                Line::from(Span::styled(
                    format!("{}s ago — reconnecting...", elapsed),
                    Style::default().fg(s::dim()),
                )),
            ]
        }
        ConnStatus::Connecting => vec![Line::from(Span::styled(
            "Connecting...",
            Style::default().fg(s::dim()),
        ))],
        ConnStatus::Connected => return,
    };

    let height = (lines.len() as u16 + 2).min(area.height);
    let toast_area = Rect::new(
        area.width.saturating_sub(width + 1),
        area.height.saturating_sub(height + 1),
        width,
        height,
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(s::error_color()))
        .style(Style::default().bg(s::error_bg()));

    let paragraph = Paragraph::new(lines)
        .style(Style::default().fg(s::error_color()))
        .block(block);

    frame.render_widget(paragraph, toast_area);
}
