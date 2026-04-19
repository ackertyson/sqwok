use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::{
    app::{ConfigureFocus, InviteModalState, InviteStep},
    style as s,
};

use super::modal::draw_modal_frame;

pub const TTL_OPTIONS: [(&str, &str); 3] =
    [("1h", "1 hour"), ("24h", "24 hours"), ("7d", "7 days")];

pub const USE_LIMIT_OPTIONS: [Option<u32>; 7] = [
    None,
    Some(1),
    Some(2),
    Some(5),
    Some(10),
    Some(25),
    Some(50),
];

fn use_limit_label(opt: Option<u32>) -> &'static str {
    match opt {
        None => "Unlimited",
        Some(1) => "1 use",
        Some(2) => "2 uses",
        Some(5) => "5 uses",
        Some(10) => "10 uses",
        Some(25) => "25 uses",
        Some(50) => "50 uses",
        _ => "Custom",
    }
}

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
            Constraint::Length(1), // error
            Constraint::Length(1), // "Expires after:" label
            Constraint::Length(3), // TTL options
            Constraint::Length(1), // spacer
            Constraint::Length(1), // "Use limit:" label
            Constraint::Length(1), // use limit value
            Constraint::Min(0),    // flexible spacer
            Constraint::Length(1), // key hints
        ])
        .split(area);

    // Error row
    if let Some(ref err) = state.error {
        frame.render_widget(
            Paragraph::new(err.as_str()).style(Style::default().fg(s::error_color())),
            chunks[0],
        );
    }

    // TTL section
    let ttl_focused = state.configure_focus == ConfigureFocus::Ttl;
    let section_style = if ttl_focused {
        Style::default().fg(s::fg())
    } else {
        Style::default().fg(s::dim())
    };
    frame.render_widget(
        Paragraph::new("Expires after:").style(section_style),
        chunks[1],
    );

    let ttl_lines: Vec<Line> = TTL_OPTIONS
        .iter()
        .enumerate()
        .map(|(i, (_, label))| {
            let selected = i == state.ttl_selection;
            let marker = if selected && ttl_focused {
                "► "
            } else {
                "  "
            };
            let style = if selected && ttl_focused {
                Style::default()
                    .fg(s::accent())
                    .add_modifier(Modifier::BOLD)
            } else if selected {
                Style::default().fg(s::fg())
            } else {
                Style::default().fg(s::dim())
            };
            Line::from(Span::styled(format!("{}{}", marker, label), style))
        })
        .collect();
    frame.render_widget(Paragraph::new(ttl_lines), chunks[2]);

    // Use limit section
    let ul_focused = state.configure_focus == ConfigureFocus::UseLimit;
    let ul_section_style = if ul_focused {
        Style::default().fg(s::fg())
    } else {
        Style::default().fg(s::dim())
    };
    frame.render_widget(
        Paragraph::new("Use limit:").style(ul_section_style),
        chunks[4],
    );

    let current = USE_LIMIT_OPTIONS[state.use_limit_idx];
    let label = use_limit_label(current);
    let ul_value_line = if ul_focused {
        Line::from(vec![
            Span::styled("► ", Style::default().fg(s::accent())),
            Span::styled(
                label,
                Style::default()
                    .fg(s::accent())
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    } else {
        Line::from(Span::styled(
            format!("  {}", label),
            Style::default().fg(s::dim()),
        ))
    };
    frame.render_widget(Paragraph::new(ul_value_line), chunks[5]);

    // Key hints
    frame.render_widget(
        Paragraph::new("↑↓ select   Tab switch field   Enter create   Esc cancel")
            .style(Style::default().fg(s::dim()))
            .alignment(Alignment::Center),
        chunks[7],
    );
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
                    Span::styled(format!("sqwok-{}", inv.display_code), style),
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
            KeyCode::Tab => {
                state.configure_focus = match state.configure_focus {
                    ConfigureFocus::Ttl => ConfigureFocus::UseLimit,
                    ConfigureFocus::UseLimit => ConfigureFocus::Ttl,
                };
                false
            }
            KeyCode::Up => {
                match state.configure_focus {
                    ConfigureFocus::Ttl => {
                        state.ttl_selection = state.ttl_selection.saturating_sub(1);
                    }
                    ConfigureFocus::UseLimit => {
                        state.use_limit_idx = state.use_limit_idx.saturating_sub(1);
                    }
                }
                false
            }
            KeyCode::Down => {
                match state.configure_focus {
                    ConfigureFocus::Ttl => {
                        state.ttl_selection = (state.ttl_selection + 1).min(TTL_OPTIONS.len() - 1);
                    }
                    ConfigureFocus::UseLimit => {
                        state.use_limit_idx =
                            (state.use_limit_idx + 1).min(USE_LIMIT_OPTIONS.len() - 1);
                    }
                }
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
