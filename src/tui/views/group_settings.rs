use ratatui::{layout::Alignment, style::Style, widgets::Paragraph, Frame};

use super::modal::draw_modal_frame;
use crate::tui::{app::AppState, style as s};

pub fn draw(frame: &mut Frame, app: &AppState) {
    draw_modal_frame(frame, "Group Settings", |frame, area| {
        let topic = app
            .chat_list
            .iter()
            .find(|c| Some(&c.uuid) == app.current_chat.as_ref())
            .map(|c| c.topic.as_str())
            .unwrap_or("—");
        let chat_uuid = app.current_chat.as_deref().unwrap_or("—");
        let member_count = app.members.len();
        let key_fp = app.my_key_fingerprint();

        // Determine our role in this chat
        let my_role = app
            .members
            .iter()
            .find(|m| m.uuid == app.my_uuid)
            .map(|_| "member") // roles aren't tracked in Member struct yet
            .unwrap_or("unknown");

        let leave_hint = if app.pending_leave_chat {
            "Leaving chat..."
        } else {
            "[L] Leave chat"
        };

        let text = format!(
            "Topic: {}\nChat ID: {}\nMembers: {}\nRole: {}\nMy key: {}\n\n[R] Rotate encryption keys\n{}\n\nEsc close",
            topic, chat_uuid, member_count, my_role, key_fp, leave_hint,
        );

        let para = Paragraph::new(text)
            .style(Style::default().fg(s::fg()))
            .alignment(Alignment::Left);
        frame.render_widget(para, area);
    });
}
