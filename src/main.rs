mod commands;
mod errors;
mod frontmatter;
mod log_filename;
mod paths;
mod templates;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::{Level, debug};

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
    /// Output new session logs for distillation
    Distill,
    /// Output soul-writing instructions for a preference
    Instill {
        /// The preference or fact to remember
        text: String,
    },
    /// Nudge about stale undistilled logs
    Nudge,
    /// Handle the Claude Code SessionEnd hook
    SessionEnd,
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

    let state_dir = paths::state_dir()?;

    match &cli.command {
        Command::AgentSetup => {
            commands::agent_setup::run(&state_dir, &mut std::io::stdout())?;
        }
        Command::Context => {
            commands::context::run(&state_dir, &mut std::io::stdout())?;
        }
        Command::Distill => {
            commands::distill::run(&state_dir, &mut std::io::stdout())?;
        }
        Command::Instill { text } => {
            commands::instill::run(&state_dir, &mut std::io::stdout(), text)?;
        }
        Command::Nudge => {
            commands::nudge::run(&state_dir, &mut std::io::stdout())?;
        }
        Command::SessionEnd => {
            commands::session_end::run(&state_dir, &mut std::io::stdin(), &mut std::io::stdout())?;
        }
        Command::SoulUpgrade => {
            commands::soul_upgrade::run(&state_dir, &mut std::io::stdout())?;
        }
    }

    Ok(())
}
