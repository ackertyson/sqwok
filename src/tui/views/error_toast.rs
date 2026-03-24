use ratatui::{
    layout::Rect,
    style::Style,
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::{
    app::{AppState, ConnStatus},
    style as s,
};

pub fn draw(frame: &mut Frame, app: &AppState) {
    let text = match &app.connection_status {
        ConnStatus::Disconnected { ref reason, since } => {
            let elapsed = since.elapsed();
            format!(
                "Disconnected\n{}\n{}s ago -- reconnecting...",
                reason,
                elapsed.as_secs()
            )
        }
        ConnStatus::Connecting => "Connecting...".to_string(),
        ConnStatus::Connected => return,
    };

    let width = 36u16;
    let height = 4u16;
    let area = frame.area();
    let toast_area = Rect::new(
        area.width.saturating_sub(width + 1),
        area.height.saturating_sub(height + 1),
        width,
        height,
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(s::error_color()))
        .style(Style::default().bg(ratatui::style::Color::Rgb(40, 15, 15)));

    let paragraph = Paragraph::new(text)
        .style(Style::default().fg(s::error_color()))
        .block(block);

    frame.render_widget(paragraph, toast_area);
}
