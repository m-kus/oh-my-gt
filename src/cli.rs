//! Argument dispatch. Commands are bare verbs; no flags are parsed.

use crate::commands;
use crate::error::{GtError, Result};
use crate::git;

const USAGE: &str = "\
gt — stacked PRs, the fast way

usage: gt <command>

stack:
  track      start tracking the current branch (pick its parent)
  untrack    stop tracking the current branch
  create     create a new branch stacked on the current one
  modify     amend the current branch and restack everything above it
  move       re-parent the current branch onto another branch
  restack    rebase the current stack so each branch sits on its parent

remote:
  submit     push the stack and create/update its pull requests
  sync       pull trunk, drop merged branches, restack survivors

rebase control:
  continue   resume after resolving conflicts
  abort      undo an in-progress operation

other:
  log        show the current stack (with commit SHAs and restack markers)
  tree       show the current stack as a focused branch tree
  help       show this message
";

pub fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let cmd = match args.next() {
        Some(c) => c,
        None => {
            print!("{USAGE}");
            return Ok(());
        }
    };

    match cmd.as_str() {
        "help" | "-h" | "--help" => {
            print!("{USAGE}");
            return Ok(());
        }
        "version" | "-V" | "--version" => {
            println!("gt {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        _ => {}
    }

    git::check_version()?;
    git::ensure_repo()?;
    crate::migrate::check()?;

    // While an operation is paused, only `continue`/`abort`/`log` are allowed.
    let needs_idle = matches!(
        cmd.as_str(),
        "track" | "untrack" | "create" | "modify" | "submit" | "move" | "restack" | "sync"
    );
    if needs_idle && crate::state::exists() {
        return Err(GtError::State(
            "an operation is in progress — run `gt continue` or `gt abort` first".into(),
        ));
    }

    match cmd.as_str() {
        "track" => commands::track::run(),
        "untrack" => commands::untrack::run(),
        "create" => commands::create::run(),
        "modify" => commands::modify::run(),
        "submit" => commands::submit::run(),
        "move" => commands::move_::run(),
        "restack" => commands::restack::run(),
        "sync" => commands::sync::run(),
        "continue" => commands::continue_::run(),
        "abort" => commands::abort::run(),
        "log" => commands::log::run(),
        "tree" => commands::tree::run(),
        other => Err(GtError::Usage(format!(
            "unknown command `{other}`\n\n{USAGE}"
        ))),
    }
}
