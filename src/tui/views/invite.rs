use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::{
    app::{InviteModalState, InviteStep},
    style as s,
};

use super::modal::draw_modal_frame;

pub const TTL_OPTIONS: [(&str, &str); 3] =
    [("1h", "1 hour"), ("24h", "24 hours"), ("7d", "7 days")];

pub fn draw(frame: &mut Frame, state: &InviteModalState) {
    draw_modal_frame(frame, "Create Invite Code", |frame, area| {
        match state.step {
            InviteStep::Configure => draw_configure(frame, area, state),
            InviteStep::Creating => draw_creating(frame, area),
            InviteStep::Display => draw_display(frame, area, state),
        }
    });
}

fn draw_configure(frame: &mut Frame, area: ratatui::layout::Rect, state: &InviteModalState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new("Expires after:").style(Style::default().fg(s::dim())),
        chunks[0],
    );

    let ttl_lines: Vec<Line> = TTL_OPTIONS
        .iter()
        .enumerate()
        .map(|(i, (_, label))| {
            let marker = if i == state.ttl_selection {
                "► "
            } else {
                "  "
            };
            let style = if i == state.ttl_selection {
                Style::default()
                    .fg(s::accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(s::fg())
            };
            Line::from(Span::styled(format!("{}{}", marker, label), style))
        })
        .collect();

    frame.render_widget(Paragraph::new(ttl_lines), chunks[2]);

    let limit_label = match state.use_limit {
        None => "Use limit: unlimited".to_string(),
        Some(n) => format!("Use limit: {} use(s)", n),
    };
    frame.render_widget(
        Paragraph::new(limit_label).style(Style::default().fg(s::dim())),
        chunks[3],
    );

    frame.render_widget(
        Paragraph::new("↑↓ select TTL   Enter create   Esc cancel")
            .style(Style::default().fg(s::dim()))
            .alignment(Alignment::Center),
        chunks[4],
    );

    if let Some(ref err) = state.error {
        let err_line = Paragraph::new(err.as_str()).style(Style::default().fg(s::error_color()));
        frame.render_widget(err_line, chunks[1]);
    }
}

fn draw_creating(frame: &mut Frame, area: ratatui::layout::Rect) {
    frame.render_widget(
        Paragraph::new("Creating invite code…")
            .style(Style::default().fg(s::dim()))
            .alignment(Alignment::Center),
        area,
    );
}

fn draw_display(frame: &mut Frame, area: ratatui::layout::Rect, state: &InviteModalState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    if let Some(ref code) = state.created_code {
        frame.render_widget(
            Paragraph::new(code.clone())
                .style(
                    Style::default()
                        .fg(s::success_color())
                        .add_modifier(Modifier::BOLD),
                )
                .alignment(Alignment::Center),
            chunks[1],
        );
    }

    frame.render_widget(
        Paragraph::new("Share this code with others.")
            .style(Style::default().fg(s::dim()))
            .alignment(Alignment::Center),
        chunks[2],
    );

    frame.render_widget(
        Paragraph::new("Enter/Esc close")
            .style(Style::default().fg(s::dim()))
            .alignment(Alignment::Center),
        chunks[3],
    );

    // Active invites list
    if !state.active_invites.is_empty() {
        let lines: Vec<Line> = state
            .active_invites
            .iter()
            .enumerate()
            .map(|(i, inv)| {
                let uses = match inv.uses_remaining {
                    Some(n) => format!("{} uses left", n),
                    None => "unlimited".to_string(),
                };
                let marker = if i == state.selected_invite {
                    "► "
                } else {
                    "  "
                };
                let style = if i == state.selected_invite {
                    Style::default()
                        .fg(s::accent())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(s::accent())
                };
                Line::from(vec![
                    Span::styled(marker, style),
                    Span::styled(inv.display_code.clone(), style),
                    Span::styled(
                        format!("  {}  exp: {}", uses, inv.expires_at),
                        Style::default().fg(s::dim()),
                    ),
                ])
            })
            .collect();
        let mut invite_lines = vec![Line::from(Span::styled(
            "Active invites (D to revoke):",
            Style::default().fg(s::dim()),
        ))];
        invite_lines.extend(lines);
        frame.render_widget(Paragraph::new(invite_lines), chunks[0]);
    }
}

pub fn handle_input(key: crossterm::event::KeyEvent, state: &mut InviteModalState) -> bool {
    use crossterm::event::KeyCode;

    match state.step {
        InviteStep::Configure => match key.code {
            KeyCode::Up => {
                state.ttl_selection = state.ttl_selection.saturating_sub(1);
                false
            }
            KeyCode::Down => {
                state.ttl_selection = (state.ttl_selection + 1).min(TTL_OPTIONS.len() - 1);
                false
            }
            KeyCode::Enter => {
                state.step = InviteStep::Creating;
                false
            }
            KeyCode::Esc => true, // close
            _ => false,
        },
        InviteStep::Display => match key.code {
            KeyCode::Enter | KeyCode::Esc => true, // close
            KeyCode::Up => {
                state.selected_invite = state.selected_invite.saturating_sub(1);
                false
            }
            KeyCode::Down => {
                if !state.active_invites.is_empty() {
                    state.selected_invite =
                        (state.selected_invite + 1).min(state.active_invites.len() - 1);
                }
                false
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                // Revoke selected invite
                if let Some(inv) = state.active_invites.get(state.selected_invite) {
                    state.pending_revoke = Some(inv.code.clone());
                }
                false
            }
            _ => false,
        },
        InviteStep::Creating => false,
    }
}
