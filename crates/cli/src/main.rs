use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "codex-unified", version, about = "Codex Unified control CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print local component health once implemented.
    Doctor,
    /// Inspect or execute migration operations.
    Migrate {
        #[command(subcommand)]
        command: MigrateCommand,
    },
}

#[derive(Debug, Subcommand)]
enum MigrateCommand {
    Inspect,
    Apply,
    Rollback,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Doctor => println!("doctor: foundation scaffold"),
        Command::Migrate { command } => println!("migrate {command:?}: not implemented"),
    }
}
