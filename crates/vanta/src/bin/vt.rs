//! `vt` — the short alias for `vanta` (identical behavior; see `docs/04-cli.md`).
#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match vanta_cli::run(&args) {
        Ok(code) => ExitCode::from(code.as_i32() as u8),
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(err.exit().as_i32() as u8)
        }
    }
}
