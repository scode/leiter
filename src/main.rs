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
    /// Claude Code hook commands
    Hook {
        #[command(subcommand)]
        command: HookCommand,
    },
    /// Claude-specific agent commands
    Claude {
        #[command(subcommand)]
        command: ClaudeCommand,
    },
    /// Soul management commands
    Soul {
        #[command(subcommand)]
        command: SoulCommand,
    },
}

#[derive(Subcommand)]
pub enum ClaudeCommand {
    /// First-time setup and hook configuration
    Install,
    /// Remove leiter hooks from Claude Code
    Uninstall,
}

#[derive(Subcommand)]
pub enum SoulCommand {
    /// Output soul-writing instructions for a preference
    Instill {
        /// The preference or fact to remember
        text: String,
    },
    /// Output new session logs for distillation
    Distill {
        /// Report obsolete files without deleting them
        #[arg(long)]
        dry_run: bool,
    },
    /// Detect and output soul template migration instructions
    Upgrade,
}

#[derive(Subcommand)]
pub enum HookCommand {
    /// Output soul content and agent instructions
    Context,
    /// Nudge about stale undistilled logs
    Nudge,
    /// Handle the Claude Code SessionEnd hook
    SessionEnd,
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
        Command::Hook { command } => match command {
            HookCommand::Context => {
                commands::context::run(&state_dir, &mut std::io::stdout())?;
            }
            HookCommand::Nudge => {
                commands::nudge::run(&state_dir, &mut std::io::stdout())?;
            }
            HookCommand::SessionEnd => {
                commands::session_end::run(
                    &state_dir,
                    &mut std::io::stdin(),
                    &mut std::io::stdout(),
                )?;
            }
        },
        Command::Soul { command } => match command {
            SoulCommand::Instill { text } => {
                commands::instill::run(&state_dir, &mut std::io::stdout(), text)?;
            }
            SoulCommand::Distill { dry_run } => {
                commands::distill::run(&state_dir, &mut std::io::stdout(), *dry_run)?;
            }
            SoulCommand::Upgrade => {
                commands::soul_upgrade::run(&state_dir, &mut std::io::stdout())?;
            }
        },
        Command::Claude { command } => match command {
            ClaudeCommand::Install => {
                commands::agent_setup::run(&state_dir, &mut std::io::stdout())?;
            }
            ClaudeCommand::Uninstall => {
                commands::agent_uninstall::run(&state_dir, &mut std::io::stdout())?;
            }
        },
    }

    Ok(())
}
