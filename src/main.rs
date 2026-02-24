use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::{debug, error, Level};

#[derive(Parser)]
#[command(name = "leiter", about = "Self-training system for Claude Code")]
pub struct Cli {
    /// Increase verbosity (-v for DEBUG, -vv for TRACE)
    #[arg(short = 'v', action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Decrease verbosity (-q for WARN, -qq for ERROR)
    #[arg(short = 'q', action = clap::ArgAction::Count, global = true)]
    quiet: u8,

    /// Set log level explicitly (overrides -v/-q)
    #[arg(long = "log-level", global = true)]
    log_level: Option<Level>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// First-time setup
    AgentSetup,
    /// Output soul content and agent instructions
    Context,
    /// Store a session log
    Log {
        /// Claude Code session ID
        #[arg(long)]
        session_id: String,
    },
    /// Output unprocessed session logs for distillation
    Distill,
    /// Handle the Claude Code Stop hook
    StopHook,
    /// Detect and output soul template migration instructions
    SoulUpgrade,
}

impl Cli {
    pub fn resolve_log_level(&self) -> Level {
        if let Some(level) = self.log_level {
            return level;
        }
        // -v takes precedence over -q when both are provided
        if self.verbose >= 2 {
            Level::TRACE
        } else if self.verbose == 1 {
            Level::DEBUG
        } else if self.quiet >= 2 {
            Level::ERROR
        } else if self.quiet == 1 {
            Level::WARN
        } else {
            Level::INFO
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let level = cli.resolve_log_level();
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_writer(std::io::stderr)
        .init();

    debug!("dispatching command");

    match &cli.command {
        Command::AgentSetup => {
            error!("agent-setup: not yet implemented");
        }
        Command::Context => {
            error!("context: not yet implemented");
        }
        Command::Log { session_id } => {
            error!(session_id, "log: not yet implemented");
        }
        Command::Distill => {
            error!("distill: not yet implemented");
        }
        Command::StopHook => {
            error!("stop-hook: not yet implemented");
        }
        Command::SoulUpgrade => {
            error!("soul-upgrade: not yet implemented");
        }
    }

    Ok(())
}
