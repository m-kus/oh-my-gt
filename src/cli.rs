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
  up         switch to a child of the current branch
  down       switch to the parent of the current branch

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
        "track"
            | "untrack"
            | "create"
            | "modify"
            | "submit"
            | "move"
            | "restack"
            | "sync"
            | "up"
            | "down"
    );
    if needs_idle {
        let live_rebase = git::rebase_in_progress(&git::git_dir()?);
        if crate::state::exists() {
            // A recorded multi-step operation. If git is no longer mid-rebase,
            // the rebase was ended out of band with native `git rebase` —
            // `git status` looks clean even though gt still holds the op. Say
            // which case it is so the contradictory states don't read as a bug.
            let msg = if live_rebase {
                "an operation is in progress — run `gt continue` or `gt abort` first"
            } else {
                "gt has a paused restack recorded, but its git rebase is no longer in progress \
                 (it was ended with native `git rebase`, so `git status` looks clean). Run \
                 `gt continue` to reconcile and finish it, or `gt abort` to discard it"
            };
            return Err(GtError::State(msg.into()));
        } else if live_rebase {
            // A git-native restack is paused (no state file). git owns it.
            return Err(GtError::State(
                "a git rebase is in progress — run `gt continue` to finish the restack or \
                 `gt abort` to undo it"
                    .into(),
            ));
        }
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
        "up" => commands::up::run(),
        "down" => commands::down::run(),
        other => Err(GtError::Usage(format!(
            "unknown command `{other}`\n\n{USAGE}"
        ))),
    }
}
