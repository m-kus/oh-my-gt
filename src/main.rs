//! oh-my-gt — a fast, low-dependency subset of the Graphite CLI for stacked PRs.
//!
//! Invoked as `gt`. Commands are bare verbs (no flags); anything the command
//! needs is collected through interactive prompts.

mod cli;
mod commands;
mod error;
mod gh;
mod git;
mod graph;
mod meta;
mod migrate;
mod prompt;
mod rebase;
mod state;
mod style;
mod tree;
mod trunk;

use error::GtError;
use style::OutputStyle;

use std::process::ExitCode;

fn main() -> ExitCode {
    match cli::run() {
        Ok(()) => ExitCode::SUCCESS,
        // A conflict pause already printed its own instructions.
        Err(GtError::Paused) => ExitCode::FAILURE,
        Err(GtError::Aborted) => {
            eprintln!("aborted");
            ExitCode::FAILURE
        }
        Err(e) => {
            let style = OutputStyle::stderr();
            eprintln!("{} {e}", style.error("error:"));
            ExitCode::FAILURE
        }
    }
}
