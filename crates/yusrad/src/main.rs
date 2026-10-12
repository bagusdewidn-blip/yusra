//! Daemon dan CLI yusra.

mod config;
mod init;
mod up;

use std::{io::IsTerminal, path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "yusrad",
    version,
    about = "Middleware MCP antara repo lokal dan agen AI"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Siapkan .yusra/ di root proyek dan tambahkan ke .git/info/exclude.
    Init {
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Jalankan daemon untuk satu proyek.
    Up {
        /// Layani satu klien MCP lewat stdin/stdout.
        #[arg(long)]
        stdio: bool,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    // stdout dipakai protokol MCP; semua log ke stderr. Level lewat YUSRA_LOG.
    // Warna ANSI hanya untuk terminal: klien MCP menyimpan stderr apa adanya.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_env_filter(
            EnvFilter::try_from_env("YUSRA_LOG").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let result = match cli.command {
        Command::Init { root } => init::run(&root),
        Command::Up { stdio, root } => up::run(&root, stdio),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}
