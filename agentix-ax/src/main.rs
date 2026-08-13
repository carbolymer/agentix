mod mcp;
mod tools;
mod tui;

use agentix_harness::{AgentLoop, AskCloud, EscalationPolicy};
use anyhow::Result;
use clap::Parser;
use mcp::McpTool;
use std::path::PathBuf;
use tools::file_ops::{ListDir, ReadFile, WriteFile};
use tools::shell::RunCommand;
use tui::{McpToolHandle, TuiConfig};

#[derive(Parser)]
#[command(
    name = "ax",
    about = "Code development agent — local-first, cloud when needed"
)]
struct Cli {
    /// Task description, or '-' to read from stdin. Omit to launch the interactive TUI.
    task: Option<String>,

    /// Local model to use for the agent loop
    #[arg(short, long, default_value = "laguna-xs-2.1", env = "AGENTIX_MODEL")]
    model: String,

    /// agentix-daemon gateway URL
    #[arg(
        short,
        long,
        default_value = "http://localhost:11430",
        env = "AGENTIX_GATEWAY_URL"
    )]
    gateway: String,

    /// Cloud model for ask_cloud escalation (provider/model routes via OpenRouter)
    #[arg(
        short,
        long,
        default_value = "moonshotai/kimi-k2",
        env = "AGENTIX_CLOUD_MODEL"
    )]
    cloud: String,

    /// MCP server command to spawn (repeatable, e.g. --mcp mcp-server)
    #[arg(long = "mcp")]
    mcp_servers: Vec<String>,

    /// Working directory for run_command [default: current directory]
    #[arg(long)]
    cwd: Option<PathBuf>,

    /// Maximum total tool calls before forcing a final answer
    #[arg(long, default_value_t = 20)]
    max_calls: usize,

    /// Disable cloud escalation (ask_cloud tool)
    #[arg(long)]
    no_cloud: bool,

    /// Disable run_command (safer for untrusted tasks)
    #[arg(long)]
    no_shell: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let cwd = match cli.cwd {
        Some(p) => p,
        None => std::env::current_dir()?,
    };

    // Spawn MCP servers before deciding headless vs TUI, since both modes need them.
    // In TUI mode we suppress stderr so it doesn't corrupt the display.
    let is_tui = cli.task.is_none();
    let mut mcp_tool_handles: Vec<McpToolHandle> = vec![];
    let mut _mcp_server_guards = vec![]; // keep Arc handles alive

    for cmd in &cli.mcp_servers {
        let (handle, defs) = mcp::spawn_and_init(cmd, !is_tui).await?;
        for def in defs {
            mcp_tool_handles.push(McpToolHandle {
                name: def.name,
                description: def.description,
                input_schema: def.input_schema,
                server: handle.clone(),
            });
        }
        _mcp_server_guards.push(handle);
    }

    if is_tui {
        // TUI mode: suppress tracing output (would corrupt the display) and launch UI.
        tui::run_tui(TuiConfig {
            model: cli.model,
            cloud: cli.cloud,
            gateway: cli.gateway,
            max_calls: cli.max_calls,
            no_cloud: cli.no_cloud,
            no_shell: cli.no_shell,
            cwd,
            mcp_handles: mcp_tool_handles,
        })
        .await
    } else {
        // Headless mode: enable tracing, run once, print answer.
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "ax=info,agentix_harness=info".into()),
            )
            .init();

        // Safety: is_tui = cli.task.is_none(), so we are in the else branch
        // where cli.task is guaranteed Some.
        let task_raw = cli.task.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "task argument required in headless mode")
        })?;
        let task = if task_raw == "-" {
            use tokio::io::AsyncReadExt;
            let mut buf = String::new();
            tokio::io::stdin().read_to_string(&mut buf).await?;
            buf
        } else {
            task_raw
        };

        // Build mcp tools for headless run.
        let mcp_tools: Vec<Box<dyn agentix_harness::Tool>> = mcp_tool_handles
            .into_iter()
            .map(|h| -> Box<dyn agentix_harness::Tool> {
                Box::new(McpTool {
                    name: h.name,
                    description: h.description,
                    input_schema: h.input_schema,
                    server: h.server,
                })
            })
            .collect();

        let mut agent = AgentLoop::new(&cli.gateway, &cli.model)
            .with_policy(EscalationPolicy {
                max_tool_calls: cli.max_calls,
                ..EscalationPolicy::default()
            })
            .with_tool(Box::new(ReadFile))
            .with_tool(Box::new(WriteFile))
            .with_tool(Box::new(ListDir));

        if !cli.no_shell {
            agent = agent.with_tool(Box::new(RunCommand::new(&cwd)));
        }

        if !cli.no_cloud {
            agent = agent.with_tool(Box::new(AskCloud::new(&cli.gateway, &cli.cloud)?));
        }

        for tool in mcp_tools {
            agent = agent.with_tool(tool);
        }

        let output = agent.run(&task).await?;

        println!("{}", output.answer);
        eprintln!(
            "[ax] tool_calls={} escalations={} interventions={}",
            output.tool_calls_made, output.escalations, output.interventions
        );

        Ok(())
    }
}
