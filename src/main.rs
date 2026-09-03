//! `fig-schema`, the command line over the schema layer.
//!
//! Installed on PATH this is also `fig schema <command>`: fig hands an action
//! it has no verb for to a `fig-<action>` program, passing every argument
//! through untouched, so the two tools compose without either knowing the
//! other is there.
//!
//! Everything it decides is decided in the library beside it. This binary reads
//! files, renders findings, and chooses an exit code.

mod cli;

use std::process::ExitCode;

fn main() -> ExitCode {
    match cli::run(std::env::args().skip(1)) {
        Ok(code) => ExitCode::from(code),
        Err(failure) => {
            if let Some(message) = failure.message() {
                eprintln!("fig-schema: {message}");
            }
            if failure.wants_usage() {
                eprintln!();
                eprint!("{}", cli::USAGE);
            }
            ExitCode::from(failure.code())
        }
    }
}
