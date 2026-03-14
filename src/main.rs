mod codex;
mod commands;
mod config;
mod errors;
mod frontmatter;
mod log_filename;
mod paths;
mod soul_validation;
mod templates;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::{Level, debug};

#[derive(Parser)]
#[command(name = "leiter", about = "Self-training system for Claude Code", version = env!("LEITER_VERSION"))]
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
    /// Configure persistent leiter settings
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Claude Code hook commands
    Hook {
        #[command(subcommand)]
        command: HookCommand,
    },
    /// Claude-specific agent commands
    Claude {
        /// Override Claude Code home directory (default: ~/.claude/)
        #[arg(long)]
        claude_home: Option<std::path::PathBuf>,

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
    /// Install leiter plugin files and initialize state
    Install,
    /// Remove leiter plugin files from Claude Code
    Uninstall,
    /// Output hook configuration instructions for the agent
    AgentSetupInstructions,
    /// Output hook removal instructions for the agent
    AgentTeardownInstructions,
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
    /// Output soul contents wrapped in XML boundary tags
    Show,
    /// Detect and output soul template migration instructions
    Upgrade,
    /// Set last_distilled to the current time
    MarkDistilled,
}

#[derive(Subcommand)]
pub enum HookCommand {
    /// Output soul content and agent instructions
    Context,
    /// Nudge about stale undistilled logs
    Nudge {
        /// Silently trigger background distillation instead of asking the user
        #[arg(long)]
        auto_distill: bool,
    },
    /// Handle the Claude Code SessionEnd hook
    SessionEnd,
}

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Set a config key to a value
    Set {
        /// Config key to update
        key: String,
        /// Config value
        value: String,
    },
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
        Command::Config { command } => match command {
            ConfigCommand::Set { key, value } => {
                commands::config::set(&state_dir, &mut std::io::stdout(), key, value)?;
            }
        },
        Command::Hook { command } => match command {
            HookCommand::Context => {
                commands::context::run(&state_dir, &mut std::io::stdout())?;
            }
            HookCommand::Nudge { auto_distill } => {
                commands::nudge::run(&state_dir, &mut std::io::stdout(), *auto_distill)?;
            }
            HookCommand::SessionEnd => {
                commands::session_end::run(&state_dir, &mut std::io::stdin())?;
            }
        },
        Command::Soul { command } => match command {
            SoulCommand::Show => {
                commands::soul_show::run(&state_dir, &mut std::io::stdout())?;
            }
            SoulCommand::Instill { text } => {
                commands::instill::run(&state_dir, &mut std::io::stdout(), text)?;
            }
            SoulCommand::Distill { dry_run } => {
                commands::distill::run(&state_dir, &mut std::io::stdout(), *dry_run)?;
            }
            SoulCommand::Upgrade => {
                commands::soul_upgrade::run(&state_dir, &mut std::io::stdout())?;
            }
            SoulCommand::MarkDistilled => {
                commands::mark_distilled::run(&state_dir, &mut std::io::stdout())?;
            }
        },
        Command::Claude {
            claude_home,
            command,
        } => {
            let claude_home = match claude_home {
                Some(p) => p.clone(),
                None => paths::default_claude_home()?,
            };
            match command {
                ClaudeCommand::Install => {
                    commands::agent_setup::run(&state_dir, &claude_home)?;
                }
                ClaudeCommand::Uninstall => {
                    commands::agent_uninstall::run(&state_dir, &claude_home)?;
                }
                ClaudeCommand::AgentSetupInstructions => {
                    commands::agent_setup::agent_setup_instructions(
                        &state_dir,
                        &mut std::io::stdout(),
                    )?;
                }
                ClaudeCommand::AgentTeardownInstructions => {
                    commands::agent_uninstall::agent_teardown_instructions(
                        &state_dir,
                        &mut std::io::stdout(),
                    )?;
                }
            }
        }
    }

    Ok(())
}
