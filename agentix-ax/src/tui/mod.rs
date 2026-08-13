mod ui;

use crate::mcp::{McpServer, McpTool};
use crate::tools::file_ops::{ListDir, ReadFile, WriteFile};
use crate::tools::shell::RunCommand;
use agentix_harness::{AgentEvent, AgentLoop, AgentOutput, AskCloud, EscalationPolicy};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{io, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{mpsc, oneshot, Mutex};

pub struct McpToolHandle {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub server: Arc<Mutex<McpServer>>,
}

pub struct TuiConfig {
    pub model: String,
    pub cloud: String,
    pub gateway: String,
    pub max_calls: usize,
    pub no_cloud: bool,
    pub no_shell: bool,
    pub cwd: PathBuf,
    pub mcp_handles: Vec<McpToolHandle>,
}

pub struct LogEntry {
    pub kind: LogKind,
    pub text: String,
}

pub enum LogKind {
    Task,
    ToolCall,
    ToolResult,
    CloudEscalation,
    Stagnation,
    Budget,
    Answer,
    Error,
}

pub enum AgentStatus {
    Idle,
    Running,
    Done,
}

pub struct App {
    pub log: Vec<LogEntry>,
    /// Lines from the bottom; 0 = follow latest.
    pub scroll: usize,
    pub input: String,
    pub cursor_pos: usize,
    pub status: AgentStatus,
    pub spinner_frame: usize,
    pub calls_made: usize,
    pub escalations_made: usize,
    pub should_quit: bool,
    agent_rx: Option<mpsc::UnboundedReceiver<AgentEvent>>,
    done_rx: Option<oneshot::Receiver<Result<AgentOutput>>>,
}

impl App {
    fn new() -> Self {
        Self {
            log: vec![],
            scroll: 0,
            input: String::new(),
            cursor_pos: 0,
            status: AgentStatus::Idle,
            spinner_frame: 0,
            calls_made: 0,
            escalations_made: 0,
            should_quit: false,
            agent_rx: None,
            done_rx: None,
        }
    }

    fn push_log(&mut self, kind: LogKind, text: impl Into<String>) {
        self.log.push(LogEntry {
            kind,
            text: text.into(),
        });
        // When following (scroll == 0), auto-scroll keeps working because
        // render always shows the last N entries at scroll=0. No state change needed.
    }
}

pub async fn run_tui(config: TuiConfig) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;

    let mut app = App::new();
    let result = event_loop(&mut terminal, &mut app, &config).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    config: &TuiConfig,
) -> Result<()> {
    let mut tick: u64 = 0;

    loop {
        // Drain crossterm key events without blocking (0-ms poll).
        while event::poll(Duration::ZERO)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    handle_key(app, key.code, key.modifiers, config);
                }
            }
        }

        // Drain agent events — collect first so the borrow on app.agent_rx ends
        // before we call process_agent_event (which needs &mut App).
        let mut agent_disconnected = false;
        let mut pending_events: Vec<AgentEvent> = vec![];
        if let Some(rx) = &mut app.agent_rx {
            loop {
                match rx.try_recv() {
                    Ok(ev) => pending_events.push(ev),
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        agent_disconnected = true;
                        break;
                    }
                }
            }
        }
        if agent_disconnected {
            app.agent_rx = None;
        }
        for ev in pending_events {
            process_agent_event(app, ev);
        }

        // Check for agent completion.
        if let Some(mut rx) = app.done_rx.take() {
            match rx.try_recv() {
                Ok(result) => process_completion(app, result),
                Err(oneshot::error::TryRecvError::Empty) => {
                    app.done_rx = Some(rx);
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    app.push_log(LogKind::Error, "✗ Agent task dropped unexpectedly");
                    app.status = AgentStatus::Idle;
                }
            }
        }

        // Advance spinner every other tick (every ~100 ms).
        tick = tick.wrapping_add(1);
        if tick.is_multiple_of(2) {
            app.spinner_frame = (app.spinner_frame + 1) % ui::SPINNER.len();
        }

        terminal.draw(|f| ui::render(f, app, config))?;

        if app.should_quit {
            break;
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    Ok(())
}

fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers, config: &TuiConfig) {
    let is_running = matches!(app.status, AgentStatus::Running);

    match (code, mods) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL)
        | (KeyCode::Char('q'), KeyModifiers::CONTROL) => {
            app.should_quit = true;
        }
        (KeyCode::Enter, _) if !is_running => {
            let task = app.input.trim().to_string();
            if !task.is_empty() {
                app.input.clear();
                app.cursor_pos = 0;
                submit_task(app, task, config);
            }
        }
        (KeyCode::Up, _) => {
            // Scroll back (increase offset from bottom), capped at log length.
            app.scroll = (app.scroll + 1).min(app.log.len());
        }
        (KeyCode::Down, _) => {
            app.scroll = app.scroll.saturating_sub(1);
        }
        (KeyCode::PageUp, _) => {
            app.scroll = (app.scroll + 10).min(app.log.len());
        }
        (KeyCode::PageDown, _) => {
            app.scroll = app.scroll.saturating_sub(10);
        }
        (KeyCode::Left, _) if !is_running => {
            app.cursor_pos = app.cursor_pos.saturating_sub(1);
        }
        (KeyCode::Right, _) if !is_running => {
            if app.cursor_pos < app.input.len() {
                app.cursor_pos += 1;
            }
        }
        (KeyCode::Home, _) if !is_running => {
            app.cursor_pos = 0;
        }
        (KeyCode::End, _) if !is_running => {
            app.cursor_pos = app.input.len();
        }
        (KeyCode::Backspace, _) if !is_running => {
            if app.cursor_pos > 0 {
                app.cursor_pos -= 1;
                app.input.remove(app.cursor_pos);
            }
        }
        (KeyCode::Delete, _) if !is_running => {
            if app.cursor_pos < app.input.len() {
                app.input.remove(app.cursor_pos);
            }
        }
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT)
            if !is_running =>
        {
            app.input.insert(app.cursor_pos, c);
            app.cursor_pos += c.len_utf8();
        }
        _ => {}
    }
}

fn submit_task(app: &mut App, task: String, config: &TuiConfig) {
    app.push_log(LogKind::Task, format!("◉ {task}"));
    app.status = AgentStatus::Running;
    app.calls_made = 0;
    app.escalations_made = 0;

    let (event_tx, event_rx) = mpsc::unbounded_channel::<AgentEvent>();
    let (done_tx, done_rx) = oneshot::channel::<Result<AgentOutput>>();
    app.agent_rx = Some(event_rx);
    app.done_rx = Some(done_rx);

    // Build the agent loop fresh for each task.
    let mut agent = AgentLoop::new(&config.gateway, &config.model).with_policy(EscalationPolicy {
        max_tool_calls: config.max_calls,
        ..EscalationPolicy::default()
    });

    agent = agent
        .with_tool(Box::new(ReadFile))
        .with_tool(Box::new(WriteFile))
        .with_tool(Box::new(ListDir));

    if !config.no_shell {
        agent = agent.with_tool(Box::new(RunCommand::new(&config.cwd)));
    }

    if !config.no_cloud {
        match AskCloud::new(&config.gateway, &config.cloud) {
            Ok(t) => agent = agent.with_tool(Box::new(t)),
            Err(e) => tracing::warn!("AskCloud init failed: {e}"),
        }
    }

    for h in &config.mcp_handles {
        agent = agent.with_tool(Box::new(McpTool {
            name: h.name.clone(),
            description: h.description.clone(),
            input_schema: h.input_schema.clone(),
            server: h.server.clone(),
        }));
    }

    tokio::spawn(async move {
        let result = agent.run_with_events(&task, event_tx).await;
        let _ = done_tx.send(result);
    });
}

fn process_agent_event(app: &mut App, event: AgentEvent) {
    match event {
        AgentEvent::ToolCallStarted { name, args_preview } => {
            app.calls_made += 1;
            app.push_log(LogKind::ToolCall, format!("⟳ {name}  {args_preview}"));
        }
        AgentEvent::ToolCallCompleted {
            name: _,
            result_preview,
        } => {
            app.push_log(LogKind::ToolResult, format!("✓ {result_preview}"));
        }
        AgentEvent::CloudEscalation { question_preview } => {
            app.calls_made += 1;
            app.escalations_made += 1;
            app.push_log(LogKind::CloudEscalation, format!("☁ {question_preview}"));
        }
        AgentEvent::StagnationDetected => {
            app.push_log(
                LogKind::Stagnation,
                "⚠ Stagnation detected — intervention injected",
            );
        }
        AgentEvent::BudgetExhausted => {
            app.push_log(
                LogKind::Budget,
                "◎ Tool budget exhausted — forcing final answer",
            );
        }
    }
}

fn process_completion(app: &mut App, result: Result<AgentOutput>) {
    app.agent_rx = None;
    match result {
        Ok(output) => {
            app.push_log(LogKind::Answer, format!("◎ {}", output.answer));
            app.push_log(
                LogKind::ToolResult,
                format!(
                    "  [{} calls · {} cloud · {} interventions]",
                    output.tool_calls_made, output.escalations, output.interventions
                ),
            );
        }
        Err(e) => {
            app.push_log(LogKind::Error, format!("✗ {e}"));
        }
    }
    app.status = AgentStatus::Done;
}
