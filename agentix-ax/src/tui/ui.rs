use ratatui::{
    layout::{Constraint, Direction, Layout, Position},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

use super::{AgentStatus, App, LogKind, TuiConfig};

pub const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn render(f: &mut Frame, app: &App, config: &TuiConfig) {
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(3),
        ])
        .split(area);

    // Title bar
    let status_suffix = match &app.status {
        AgentStatus::Idle => "  idle".to_string(),
        AgentStatus::Running => format!("  {} calls", app.calls_made),
        AgentStatus::Done => "  done".to_string(),
    };
    let no_cloud_tag = if config.no_cloud { "" } else { " → " };
    let cloud_part = if config.no_cloud {
        ""
    } else {
        config.cloud.as_str()
    };
    let title = format!(
        " ax  {}{}{}  |{}",
        config.model, no_cloud_tag, cloud_part, status_suffix
    );
    f.render_widget(
        Paragraph::new(title).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        chunks[0],
    );

    // Log area — scroll from bottom (scroll=0 means show latest)
    let log_height = chunks[1].height as usize;
    let total = app.log.len();
    let visible_start = if total > log_height {
        let max_offset = total - log_height;
        max_offset.saturating_sub(app.scroll)
    } else {
        0
    };

    let items: Vec<ListItem> = app
        .log
        .iter()
        .skip(visible_start)
        .take(log_height)
        .map(|entry| {
            let color = match entry.kind {
                LogKind::Task => Color::Cyan,
                LogKind::ToolCall => Color::Yellow,
                LogKind::ToolResult => Color::Green,
                LogKind::CloudEscalation => Color::LightBlue,
                LogKind::Stagnation | LogKind::Budget => Color::Magenta,
                LogKind::Answer => Color::LightGreen,
                LogKind::Error => Color::Red,
            };
            ListItem::new(Line::from(Span::styled(
                &entry.text,
                Style::default().fg(color),
            )))
        })
        .collect();

    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::NONE)),
        chunks[1],
    );

    // Activity / hint line
    let hint = match &app.status {
        AgentStatus::Idle => {
            Paragraph::new(" Idle — type a task and press Enter  |  Ctrl+C to quit")
                .style(Style::default().fg(Color::DarkGray))
        }
        AgentStatus::Running => {
            let sp = SPINNER[app.spinner_frame];
            Paragraph::new(format!(
                " {} Working…  ({} calls, {} cloud escalations)",
                sp, app.calls_made, app.escalations_made
            ))
            .style(Style::default().fg(Color::Yellow))
        }
        AgentStatus::Done => Paragraph::new(" Done — type another task or Ctrl+C to quit")
            .style(Style::default().fg(Color::Green)),
    };
    f.render_widget(hint, chunks[2]);

    // Input box
    let is_running = matches!(app.status, AgentStatus::Running);
    let border_style = if is_running {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::Cyan)
    };
    let input_display = format!("> {}", app.input);
    f.render_widget(
        Paragraph::new(input_display.as_str())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Task ")
                    .border_style(border_style),
            )
            .wrap(Wrap { trim: false }),
        chunks[3],
    );

    // Cursor (only when input is active)
    if !is_running {
        let inner_w = chunks[3].width.saturating_sub(4) as usize; // 2 borders + "> "
        if app.cursor_pos <= inner_w {
            f.set_cursor_position(Position {
                x: chunks[3].x + 1 + 2 + app.cursor_pos as u16,
                y: chunks[3].y + 1,
            });
        }
    }
}
