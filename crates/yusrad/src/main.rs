//! Daemon dan CLI yusra. Perintah diisi bertahap sesuai milestone M0.

use std::process::ExitCode;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("--version" | "-V") => {
            println!("yusrad {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!(
                "yusrad {}: belum ada perintah yang diimplementasikan",
                env!("CARGO_PKG_VERSION")
            );
            ExitCode::from(2)
        }
    }
}
