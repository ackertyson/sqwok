use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph},
    Frame,
};

use super::modal::draw_modal_frame;
use crate::tui::{app::AppState, style as s};

pub fn draw(frame: &mut Frame, app: &AppState) {
    draw_modal_frame(frame, "Members", |frame, area| {
        let items: Vec<ListItem> = app
            .members
            .iter()
            .map(|m| {
                let status = if m.online { "●" } else { "○" };
                let status_color = if m.online {
                    s::success_color()
                } else {
                    s::dim()
                };
                ListItem::new(Line::from(vec![
                    Span::styled(status, Style::default().fg(status_color)),
                    Span::raw(" "),
                    Span::styled(
                        m.screenname.clone(),
                        Style::default().fg(s::username_color(&m.uuid)),
                    ),
                ]))
            })
            .collect();

        let list = List::new(items);
        frame.render_widget(list, area);

        // Help text at bottom
        if area.height > 2 {
            let help_area = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
            let help = Paragraph::new("Esc close").style(Style::default().fg(s::dim()));
            frame.render_widget(help, help_area);
        }
    });
}
