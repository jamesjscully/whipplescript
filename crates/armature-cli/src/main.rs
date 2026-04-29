use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use armature_core::{load_workspace_config, resolve_workspace, ArmatureError, ArmatureResult};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> ArmatureResult<()> {
    let cli = Cli::parse();
    cli.command.execute(cli.workspace)
}

#[derive(Debug, Parser)]
#[command(
    name = "armature",
    version,
    about = "Local daemon for triggering and supervising ordinary programs"
)]
struct Cli {
    #[arg(long, global = true)]
    workspace: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Init(InitArgs),
    Dev,
    Up,
    Down,
    Restart,
    Run(RunArgs),
    Emit(EmitArgs),
    Status,
    Ps,
    Tasks,
    Services,
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    Runs,
    Logs(LogsArgs),
    Cancel(CancelArgs),
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Doctor,
    Lock {
        #[command(subcommand)]
        command: LockCommand,
    },
}

impl Command {
    fn execute(self, workspace: Option<PathBuf>) -> ArmatureResult<()> {
        match self {
            Self::Init(args) => {
                let _ = args;
                Err(ArmatureError::not_implemented("init"))
            }
            Self::Dev => Err(ArmatureError::not_implemented("dev")),
            Self::Up => Err(ArmatureError::not_implemented("up")),
            Self::Down => Err(ArmatureError::not_implemented("down")),
            Self::Restart => Err(ArmatureError::not_implemented("restart")),
            Self::Run(_) => Err(ArmatureError::not_implemented("run")),
            Self::Emit(_) => Err(ArmatureError::not_implemented("emit")),
            Self::Status => Err(ArmatureError::not_implemented("status")),
            Self::Ps => Err(ArmatureError::not_implemented("ps")),
            Self::Tasks => Err(ArmatureError::not_implemented("tasks")),
            Self::Services => Err(ArmatureError::not_implemented("services")),
            Self::Service { .. } => Err(ArmatureError::not_implemented("service")),
            Self::Runs => Err(ArmatureError::not_implemented("runs")),
            Self::Logs(_) => Err(ArmatureError::not_implemented("logs")),
            Self::Cancel(_) => Err(ArmatureError::not_implemented("cancel")),
            Self::Config { command } => command.execute(workspace),
            Self::Doctor => Err(ArmatureError::not_implemented("doctor")),
            Self::Lock { .. } => Err(ArmatureError::not_implemented("lock")),
        }
    }
}

impl ConfigCommand {
    fn execute(self, workspace: Option<PathBuf>) -> ArmatureResult<()> {
        match self {
            Self::Check => {
                let cwd = std::env::current_dir()?;
                let workspace = resolve_workspace(workspace.as_ref(), &cwd)?;
                let config = load_workspace_config(&workspace)?;
                println!(
                    "ok {} {}",
                    workspace.config_path().display(),
                    config.version
                );
                Ok(())
            }
        }
    }
}

#[derive(Debug, Args)]
struct InitArgs {
    #[command(subcommand)]
    command: Option<InitCommand>,
}

#[derive(Debug, Subcommand)]
enum InitCommand {
    Recipe(RecipeArgs),
}

#[derive(Debug, Args)]
struct RecipeArgs {
    name: String,
}

#[derive(Debug, Args)]
struct RunArgs {
    task_name: String,
}

#[derive(Debug, Args)]
struct EmitArgs {
    event_type: String,
    #[arg(long)]
    json: Option<String>,
}

#[derive(Debug, Args)]
struct LogsArgs {
    run_id: String,
}

#[derive(Debug, Args)]
struct CancelArgs {
    run_id: String,
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    Check,
}

#[derive(Debug, Subcommand)]
enum ServiceCommand {
    Start(ServiceNameArgs),
    Stop(ServiceNameArgs),
    Restart(ServiceNameArgs),
}

#[derive(Debug, Args)]
struct ServiceNameArgs {
    name: String,
}

#[derive(Debug, Subcommand)]
enum LockCommand {
    Acquire(LockNameArgs),
    Release(LockNameArgs),
    Status,
}

#[derive(Debug, Args)]
struct LockNameArgs {
    name: String,
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    #[test]
    fn cli_command_tree_builds() {
        Cli::command().debug_assert();
    }
}
