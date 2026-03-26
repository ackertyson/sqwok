use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::Style,
    widgets::{Block, Borders},
    Frame,
};

use super::{
    app::{AppState, ConnStatus, ModalState, Mode, PaneSplit},
    style as s, views,
};

pub fn draw(frame: &mut Frame, app: &mut AppState) {
    match app.mode {
        Mode::ChatPicker => views::chat_picker::draw(frame, app),
        Mode::Chat => draw_chat(frame, app),
    }

    // Connection error overlay — shown in all modes so startup failures are visible.
    if matches!(
        app.connection_status,
        ConnStatus::Disconnected { .. } | ConnStatus::Connecting
    ) {
        views::error_toast::draw(frame, app);
    }
}

fn draw_chat(frame: &mut Frame, app: &mut AppState) {
    let area = frame.area();

    // Outer 3-row layout: top bar, pane area, bottom bar
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    views::chat::draw_top_bar(frame, outer[0], app);
    views::chat::draw_bottom_bar(frame, outer[2], app);

    let pane_area = outer[1];

    if app.panes.len() == 1 {
        let pane_snap = app.panes[0].clone();
        views::chat::draw_messages(frame, pane_area, app, &pane_snap);
    } else {
        let pane_count = app.panes.len();
        let active_pane_idx = app.active_pane;

        let split_direction = match app.pane_split {
            PaneSplit::Vertical => Direction::Horizontal,
            PaneSplit::Horizontal => Direction::Vertical,
        };
        let chunks = Layout::default()
            .direction(split_direction)
            .constraints(
                (0..pane_count)
                    .map(|_| Constraint::Ratio(1, pane_count as u32))
                    .collect::<Vec<_>>(),
            )
            .split(pane_area);

        let pane_snapshots: Vec<_> = app.panes.to_vec();

        for (i, pane_snap) in pane_snapshots.iter().enumerate() {
            let chunk = chunks[i];
            let border_style = if i == active_pane_idx {
                Style::default().fg(s::accent())
            } else {
                Style::default().fg(s::dim())
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(border_style);
            let inner = block.inner(chunk);
            frame.render_widget(block, chunk);
            views::chat::draw_messages(frame, inner, app, pane_snap);
        }
    }

    // Overlays — for variants that pass &AppState to draw, release the borrow
    // first by checking via matches! before calling into the view.
    let is_member_list = matches!(app.modal, Some(ModalState::MemberList));
    let is_group_settings = matches!(app.modal, Some(ModalState::GroupSettings));
    if is_member_list {
        views::member_list::draw(frame, app);
    } else if is_group_settings {
        views::group_settings::draw(frame, app);
    } else {
        match &app.modal {
            Some(ModalState::InviteCreate(s)) => {
                let s_clone = s.clone();
                views::invite::draw(frame, &s_clone);
            }
            Some(ModalState::Search(s)) => views::search::draw(frame, s),
            Some(ModalState::Contacts(s)) => views::contacts::draw(frame, s),
            _ => {}
        }
    }

    if app.command_bar.is_some() {
        let cmd_clone = app.command_bar.as_ref().unwrap().clone();
        views::command_bar::draw(frame, &cmd_clone);
    }

    // Toast notification overlay (bottom-right)
    if let Some((ref msg, _)) = app.toast {
        let width = (msg.len() as u16 + 4).min(area.width);
        let toast_area = ratatui::layout::Rect::new(
            area.width.saturating_sub(width + 1),
            area.height.saturating_sub(2),
            width,
            1,
        );
        let toast = ratatui::widgets::Paragraph::new(msg.as_str()).style(
            Style::default()
                .fg(s::success_color())
                .bg(ratatui::style::Color::Rgb(15, 30, 15)),
        );
        frame.render_widget(toast, toast_area);
    }
}
