//! Minimal interactive prompts read from stdin. Commands take no flags, so any
//! input a command needs is gathered here.

use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{GtError, Result};
use crate::tree::TreeLine;

/// Read one line from stdin and trim surrounding whitespace. Suitable for
/// keyword answers (yes/no) and numeric pickers, where leading/trailing
/// whitespace is never meaningful.
fn read_line() -> Result<String> {
    Ok(read_line_preserving()?.trim().to_string())
}

/// Read one line from stdin, stripping only the trailing line terminator
/// (`\n` or `\r\n`). Surrounding whitespace is preserved so the caller can
/// decide whether to treat it as significant.
fn read_line_preserving() -> Result<String> {
    let mut s = String::new();
    let n = io::stdin().lock().read_line(&mut s)?;
    if n == 0 {
        // EOF with no answer — treat as a declined prompt.
        return Err(GtError::Aborted);
    }
    if s.ends_with('\n') {
        s.pop();
        if s.ends_with('\r') {
            s.pop();
        }
    }
    Ok(s)
}

/// Ask a yes/no question; an empty answer takes `default`.
///
/// On an interactive TTY this resolves on a single `y`/`Y`/`n`/`N` keystroke
/// without the user needing to press Enter. Enter alone takes the default;
/// Ctrl-C (and EOF) aborts; any other key is ignored. When stdin is not a TTY
/// (tests, the e2e harness, piped input), the prompt falls back to the
/// historical line-based read so `printf 'y\n' | gt …` keeps working.
pub fn confirm(question: &str, default: bool) -> Result<bool> {
    let hint = if default { "[Y/n]" } else { "[y/N]" };
    print!("{question} {hint} ");
    io::stdout().flush()?;

    // Non-interactive: keep the legacy line-based answer. Piped input from
    // tests and the e2e harness must keep working unchanged.
    if !io::stdin().is_terminal() {
        let ans = read_line()?.to_lowercase();
        return Ok(match ans.as_str() {
            "" => default,
            "y" | "yes" => true,
            "n" | "no" => false,
            _ => default,
        });
    }

    #[cfg(unix)]
    {
        confirm_single_key(default)
    }
    #[cfg(not(unix))]
    {
        // Non-unix targets are not officially supported (the repo is
        // POSIX-flavored). Preserve the line-based behavior just in case.
        let ans = read_line()?.to_lowercase();
        Ok(match ans.as_str() {
            "" => default,
            "y" | "yes" => true,
            "n" | "no" => false,
            _ => default,
        })
    }
}

/// One-keystroke resolution of a yes/no prompt; runs only when stdin is a TTY.
///
/// Implementation notes:
///
/// * Allowed crate dependencies are `serde` / `serde_json` only — no `libc`,
///   so termios is driven by shelling out to POSIX `stty`. AGENTS.md routes
///   `git` and `gh` through their respective single chokepoints; `stty` is
///   the documented exception and is spawned directly from this module.
/// * `stty -g` prints a portable encoding of the current terminal mode; we
///   stash it on the [`TermiosGuard`] and `stty <encoding>` restores it on
///   every exit path (normal return, error, panic unwind). Both `stty` calls
///   inherit stdin so they target the controlling terminal.
/// * In raw mode the kernel does not translate `^C` into SIGINT; it just
///   delivers the literal byte `0x03`. We treat `0x03` (and `0x04`, EOT) as
///   [`GtError::Aborted`] to match the cooked-mode semantics.
#[cfg(unix)]
fn confirm_single_key(default: bool) -> Result<bool> {
    // If we cannot set the terminal up for single-key reads, fall back to the
    // line-based path so the prompt still works rather than failing outright.
    let Some(_guard) = TermiosGuard::enable_raw() else {
        let ans = read_line()?.to_lowercase();
        return Ok(match ans.as_str() {
            "" => default,
            "y" | "yes" => true,
            "n" | "no" => false,
            _ => default,
        });
    };

    let mut stdin = io::stdin().lock();
    let mut buf = [0u8; 1];
    loop {
        match stdin.read(&mut buf) {
            Ok(0) => return Err(GtError::Aborted), // EOF
            Ok(_) => {}
            Err(_) => return Err(GtError::Aborted),
        }
        match decide(buf[0], default) {
            KeyDecision::Yes => {
                // Raw mode suppresses echo; print the resolution ourselves so
                // the prompt visibly advances, mirroring Graphite's feel.
                println!("y");
                return Ok(true);
            }
            KeyDecision::No => {
                println!("n");
                return Ok(false);
            }
            KeyDecision::Default => {
                println!("{}", if default { 'y' } else { 'n' });
                return Ok(default);
            }
            KeyDecision::Abort => return Err(GtError::Aborted),
            KeyDecision::Ignore => continue,
        }
    }
}

/// Decision a single byte from stdin maps to in single-key confirm mode.
/// Split out so it can be unit-tested without involving a real terminal.
#[derive(Debug, PartialEq, Eq)]
enum KeyDecision {
    Yes,
    No,
    Default,
    Abort,
    Ignore,
}

#[cfg_attr(not(unix), allow(dead_code))]
fn decide(byte: u8, _default: bool) -> KeyDecision {
    match byte {
        b'y' | b'Y' => KeyDecision::Yes,
        b'n' | b'N' => KeyDecision::No,
        b'\n' | b'\r' => KeyDecision::Default,
        0x03 | 0x04 => KeyDecision::Abort, // Ctrl-C, Ctrl-D in raw mode
        _ => KeyDecision::Ignore,
    }
}

/// RAII helper that puts the terminal into raw mode and restores the prior
/// mode on drop. Constructed only when stdin is a TTY; `enable_raw` returns
/// `None` if `stty` is unavailable or fails so the caller can fall back to
/// line-based reading.
///
/// We capture the original mode with `stty -g` (a portable encoding) before
/// switching to raw mode, then feed that exact string back through `stty` on
/// drop. Both invocations inherit stdin so they act on the controlling
/// terminal without needing `-F /dev/tty`.
#[cfg(unix)]
struct TermiosGuard {
    saved: String,
}

#[cfg(unix)]
impl TermiosGuard {
    fn enable_raw() -> Option<Self> {
        let saved = stty_capture()?;
        // `-icanon` disables line buffering, `-echo` suppresses echo, and
        // `min 1 time 0` makes `read` return as soon as one byte is available.
        if !stty_apply(&["-icanon", "-echo", "min", "1", "time", "0"]) {
            // Best effort: try to restore the saved mode in case stty applied
            // a partial change before failing.
            let _ = stty_apply(&[saved.as_str()]);
            return None;
        }
        Some(TermiosGuard { saved })
    }
}

#[cfg(unix)]
impl Drop for TermiosGuard {
    fn drop(&mut self) {
        // Best-effort restore: if this fails the user could be left in a
        // weird state, but there is nothing meaningful we can do here.
        let _ = stty_apply(&[self.saved.as_str()]);
    }
}

// `stty` is the documented exception to the "all spawns through git.rs/gh.rs"
// rule (see AGENTS.md / issue #29). The two helpers below are the only places
// in the crate that run it. They inherit stdin so stty targets the controlling
// terminal; stdout is captured for `-g` and silenced for apply.
#[cfg(unix)]
fn stty_capture() -> Option<String> {
    let out = Command::new("stty").arg("-g").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let trimmed = s.trim().to_string();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed)
}

#[cfg(unix)]
fn stty_apply(args: &[&str]) -> bool {
    Command::new("stty")
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Free-text input; an empty answer takes `default` when one is given.
///
/// Surrounding whitespace is trimmed — suitable for human prose like commit
/// messages. For branch names, use [`input_branch_name`] instead so the
/// caller's keystrokes are preserved verbatim.
pub fn input(prompt: &str, default: Option<&str>) -> Result<String> {
    input_inner(prompt, default, /* preserve */ false)
}

/// Collect a commit message from the user.
///
/// When stdin is a real terminal, launch the user's editor (`$VISUAL`, then
/// `$EDITOR`, falling back to `vi`) against a temporary file pre-seeded with
/// git-style comment lines. On editor success, comment lines are stripped and
/// the remaining message is trimmed; an empty result becomes a clear error.
///
/// When stdin is *not* a terminal (e.g. the e2e harness pipes the message in),
/// fall back to a single-line stdin read so non-interactive callers keep
/// working without needing a flag.
pub fn editor_message(prompt: &str) -> Result<String> {
    if !io::stdin().is_terminal() {
        // Non-interactive caller (tests, scripts) — preserve the historical
        // behavior of reading one trimmed line.
        return input(prompt, None);
    }
    editor_message_interactive()
}

/// Drive the configured editor against a fresh template file and return the
/// parsed message. Split out so the interactive path stays readable and the
/// cleanup happens in exactly one place.
fn editor_message_interactive() -> Result<String> {
    let path = make_temp_path("COMMIT_EDITMSG");
    std::fs::write(&path, EDITOR_TEMPLATE)?;

    let result = run_editor(&path).and_then(|()| {
        let raw = std::fs::read_to_string(&path)?;
        parse_editor_message(&raw)
    });

    // Best-effort cleanup: the temp file lives under the OS temp dir, so a
    // failure to remove it is not worth aborting the user's command over.
    let _ = std::fs::remove_file(&path);

    result
}

/// Pre-seeded template for the editor buffer. Mirrors the look of git's own
/// `COMMIT_EDITMSG` so the experience feels familiar.
const EDITOR_TEMPLATE: &str = "\n\
    # Please enter the commit message for your branch.\n\
    # Lines starting with '#' will be ignored, and an empty message aborts\n\
    # the command.\n";

/// Spawn the configured editor against `path`, inheriting the terminal so the
/// user can interact with it. A non-zero exit is treated as cancellation.
fn run_editor(path: &Path) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| "vi".to_string());

    // Use the shell to honor editor settings like `code --wait` that bake
    // arguments into $EDITOR — matches git's own behavior. The temp path is
    // appended as a separate argv entry so its spaces are safe.
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$@\""))
        .arg(&editor) // becomes $0
        .arg(path)
        .status()
        .map_err(|e| GtError::Usage(format!("failed to launch editor `{editor}`: {e}")))?;

    if !status.success() {
        return Err(GtError::Aborted);
    }
    Ok(())
}

/// Strip `#`-prefixed comment lines and surrounding whitespace; reject an
/// empty result with a clear error. Pulled out so it can be unit-tested
/// without spawning an editor.
fn parse_editor_message(raw: &str) -> Result<String> {
    let mut out = String::new();
    for line in raw.lines() {
        // Git's rule: a leading `#` (no surrounding whitespace stripped first)
        // marks a comment. Keeping the same rule means users can paste git
        // commit templates here verbatim.
        if line.starts_with('#') {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    let trimmed = out.trim().to_string();
    if trimmed.is_empty() {
        return Err(GtError::Usage("empty commit message — aborting".into()));
    }
    Ok(trimmed)
}

/// Build a unique path under `std::env::temp_dir()`. We avoid the `tempfile`
/// crate to keep the dependency footprint at `serde`/`serde_json` only.
fn make_temp_path(label: &str) -> PathBuf {
    let pid = std::process::id();
    // Nanos since the epoch is plenty of entropy for a single-process tool.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("oh-my-gt-{pid}-{nanos}-{label}"))
}

/// Prompt for a branch name, preserving the user's input exactly (only the
/// trailing line terminator is stripped). Falls back to `default` only when
/// the user presses Enter without typing anything.
///
/// Validates the result against the same rules `git check-ref-format` would
/// apply, so an invalid name produces a clear `Usage` error here instead of a
/// surprising `git branch` failure (or a silently-normalized name) later.
pub fn input_branch_name(prompt: &str, default: Option<&str>) -> Result<String> {
    let name = input_inner(prompt, default, /* preserve */ true)?;
    validate_branch_name(&name)?;
    Ok(name)
}

fn input_inner(prompt: &str, default: Option<&str>, preserve: bool) -> Result<String> {
    match default {
        Some(d) => print!("{prompt} [{d}]: "),
        None => print!("{prompt}: "),
    }
    io::stdout().flush()?;
    let ans = if preserve {
        read_line_preserving()?
    } else {
        read_line()?
    };
    if ans.is_empty() {
        match default {
            Some(d) => Ok(d.to_string()),
            None => Err(GtError::Usage("a value is required".into())),
        }
    } else {
        Ok(ans)
    }
}

/// Reject branch names that git itself would refuse. Whitespace, control
/// characters, and the specific tokens listed in `git check-ref-format` all
/// produce a clear `Usage` error rather than being silently massaged.
fn validate_branch_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(GtError::Usage("branch name must not be empty".into()));
    }
    // git check-ref-format forbids whitespace and ASCII control characters in
    // any path component. Catching these here gives a single, predictable
    // error site instead of a `git branch` failure mid-command.
    if let Some(bad) = name
        .chars()
        .find(|c| c.is_whitespace() || c.is_ascii_control())
    {
        return Err(GtError::Usage(format!(
            "branch name `{name}` contains an invalid character ({bad:?})"
        )));
    }
    // The remaining checks mirror `git check-ref-format`. They are cheap and
    // give a stable error message without spawning a process.
    const FORBIDDEN_CHARS: &[char] = &['~', '^', ':', '?', '*', '[', '\\'];
    if let Some(bad) = name.chars().find(|c| FORBIDDEN_CHARS.contains(c)) {
        return Err(GtError::Usage(format!(
            "branch name `{name}` contains an invalid character ({bad:?})"
        )));
    }
    if name.starts_with('-')
        || name.starts_with('/')
        || name.starts_with('.')
        || name.ends_with('/')
        || name.ends_with('.')
        || name.ends_with(".lock")
        || name.contains("..")
        || name.contains("//")
        || name.contains("@{")
        || name == "@"
    {
        return Err(GtError::Usage(format!(
            "branch name `{name}` is not a valid git ref"
        )));
    }
    Ok(())
}

/// Tree picker: prints `lines` as a stack tree and accepts a numeric choice
/// or a typed branch name. Only `TreeLine`s with `selectable == true` are
/// pickable; the rest render for shape only. Returns the chosen branch.
pub fn select_tree(prompt: &str, lines: &[TreeLine], default_branch: &str) -> Result<String> {
    let mut choices: Vec<&TreeLine> = lines.iter().filter(|l| l.selectable).collect();
    if choices.is_empty() {
        return Err(GtError::State("nothing to choose from".into()));
    }
    if choices.len() == 1 {
        return Ok(choices.remove(0).branch.clone());
    }
    let default = choices
        .iter()
        .position(|l| l.branch == default_branch)
        .unwrap_or(0);

    // Render: numbered slots line up under the same column whether or not a
    // row is pickable, so the tree shape stays legible.
    let width = choices.len().to_string().len();
    let blank = " ".repeat(width + 2); // "N) " worth of padding
    println!("{prompt}");
    let mut n = 0usize;
    for line in lines {
        if line.selectable {
            let marker = if n == default { ">" } else { " " };
            let num = format!("{:>width$})", n + 1, width = width);
            println!("  {marker} {num} {}", line.text);
            n += 1;
        } else {
            println!("    {blank} {}", line.text);
        }
    }
    loop {
        print!(
            "choose [1-{}, default {}, or branch name]: ",
            choices.len(),
            default + 1
        );
        io::stdout().flush()?;
        let ans = read_line()?;
        if ans.is_empty() {
            return Ok(choices[default].branch.clone());
        }
        if let Ok(idx) = ans.parse::<usize>() {
            if (1..=choices.len()).contains(&idx) {
                return Ok(choices[idx - 1].branch.clone());
            }
        }
        if let Some(c) = choices.iter().find(|l| l.branch == ans) {
            return Ok(c.branch.clone());
        }
        println!("invalid choice");
    }
}

/// Pick one option from a list; returns the chosen index.
pub fn select(prompt: &str, options: &[String], default: usize) -> Result<usize> {
    if options.is_empty() {
        return Err(GtError::State("nothing to choose from".into()));
    }
    if options.len() == 1 {
        return Ok(0);
    }
    let default = default.min(options.len() - 1);
    println!("{prompt}");
    for (i, opt) in options.iter().enumerate() {
        let marker = if i == default { ">" } else { " " };
        println!("  {marker} {}) {opt}", i + 1);
    }
    loop {
        print!("choose [1-{}, default {}]: ", options.len(), default + 1);
        io::stdout().flush()?;
        let ans = read_line()?;
        if ans.is_empty() {
            return Ok(default);
        }
        if let Ok(n) = ans.parse::<usize>() {
            if (1..=options.len()).contains(&n) {
                return Ok(n - 1);
            }
        }
        if let Some(idx) = options.iter().position(|o| o == &ans) {
            return Ok(idx);
        }
        println!("invalid choice");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_plain_names() {
        validate_branch_name("feature").unwrap();
        validate_branch_name("user/feature-1").unwrap();
        validate_branch_name("feature.x").unwrap();
        validate_branch_name("a").unwrap();
    }

    #[test]
    fn validate_rejects_empty() {
        let e = validate_branch_name("").unwrap_err();
        assert!(matches!(e, GtError::Usage(ref m) if m.contains("must not be empty")));
    }

    #[test]
    fn validate_rejects_surrounding_whitespace() {
        // The whole point of issue #12: typing "  feature" must not be
        // silently turned into "feature" — we surface a clear error and let
        // the user retype.
        let e = validate_branch_name("  feature").unwrap_err();
        assert!(
            matches!(&e, GtError::Usage(m) if m.contains("invalid character")),
            "got {e:?}"
        );
        let e = validate_branch_name("feature  ").unwrap_err();
        assert!(
            matches!(&e, GtError::Usage(m) if m.contains("invalid character")),
            "got {e:?}"
        );
    }

    #[test]
    fn validate_rejects_internal_whitespace() {
        // git refuses spaces anywhere in a ref; we follow suit explicitly
        // rather than silently mangling the name.
        for bad in [
            "feature branch",
            "feature\tbranch",
            "feature\nbranch",
            "\u{00A0}feature",
        ] {
            let e = validate_branch_name(bad).unwrap_err();
            assert!(
                matches!(&e, GtError::Usage(m) if m.contains("invalid character")),
                "input {bad:?} produced {e:?}"
            );
        }
    }

    #[test]
    fn validate_rejects_git_forbidden_tokens() {
        for bad in [
            "-leading-dash",
            "/leading-slash",
            ".leading-dot",
            "trailing-slash/",
            "trailing-dot.",
            "branch.lock",
            "a..b",
            "a//b",
            "feat@{0}",
            "@",
            "feat^",
            "feat~1",
            "feat:thing",
            "feat?",
            "feat*",
            "feat[1]",
            "feat\\back",
        ] {
            let e = validate_branch_name(bad).unwrap_err();
            assert!(
                matches!(&e, GtError::Usage(_)),
                "input {bad:?} produced {e:?}"
            );
        }
    }

    /// Mirror `read_line_preserving` against an in-memory buffer so we can
    /// exercise the preserve/trim contract without touching real stdin.
    fn read_preserving_from(bytes: &[u8]) -> String {
        let mut s = String::new();
        std::io::BufRead::read_line(&mut std::io::BufReader::new(bytes), &mut s).unwrap();
        if s.ends_with('\n') {
            s.pop();
            if s.ends_with('\r') {
                s.pop();
            }
        }
        s
    }

    #[test]
    fn read_preserving_strips_only_line_terminator() {
        // Surrounding whitespace must be preserved verbatim — exactly the
        // behavior issue #12 demands for branch-name input.
        assert_eq!(read_preserving_from(b"  feature  \n"), "  feature  ");
        assert_eq!(read_preserving_from(b"feature\r\n"), "feature");
        assert_eq!(read_preserving_from(b"feature\n"), "feature");
        // No terminator at EOF is fine, too.
        assert_eq!(read_preserving_from(b"feature"), "feature");
    }

    #[test]
    fn parse_editor_strips_comments_and_trims() {
        // Mixed body + comment lines: comments drop out, surrounding blank
        // lines are trimmed, the message body is preserved verbatim.
        let raw = "\nFix login flow\n\nResolves the regression.\n\
                   # Please enter the commit message for your branch.\n\
                   # Lines starting with '#' will be ignored.\n";
        let parsed = parse_editor_message(raw).unwrap();
        assert_eq!(parsed, "Fix login flow\n\nResolves the regression.");
    }

    #[test]
    fn parse_editor_keeps_indented_hashes() {
        // Only a leading `#` (column 0) is a comment. An indented `#` is part
        // of the body — matches git's own rule, so pasting "#42 in markdown"
        // works as long as the user indents it.
        let raw = "Title\n\n See #42 for context.\n";
        let parsed = parse_editor_message(raw).unwrap();
        assert_eq!(parsed, "Title\n\n See #42 for context.");
    }

    #[test]
    fn parse_editor_rejects_empty() {
        // Whitespace-only and comments-only buffers both count as empty —
        // the user got the editor up and decided not to write anything.
        for raw in [
            "",
            "\n\n  \n",
            "# only comments\n# more comments\n",
            "   \n\t\n",
        ] {
            let e = parse_editor_message(raw).unwrap_err();
            assert!(
                matches!(&e, GtError::Usage(m) if m.contains("empty commit message")),
                "input {raw:?} produced {e:?}"
            );
        }
    }

    #[test]
    fn parse_editor_trims_trailing_whitespace() {
        // Trailing blank lines (common after editor save) are stripped but
        // interior blank lines (e.g. between subject and body) survive.
        let raw = "Subject line\n\nBody paragraph.\n\n\n";
        let parsed = parse_editor_message(raw).unwrap();
        assert_eq!(parsed, "Subject line\n\nBody paragraph.");
    }

    #[test]
    fn read_trimming_strips_surrounding_whitespace() {
        // The yes/no and numeric picker paths still want trim semantics, so
        // keyword answers like " y " keep working.
        let trimmed: String = read_preserving_from(b"   y   \n").trim().to_string();
        assert_eq!(trimmed, "y");
    }

    #[test]
    fn decide_recognises_yes_keys() {
        // Both cases map to Yes regardless of the default — a typed `y` is
        // an explicit answer, not a hint.
        assert_eq!(decide(b'y', true), KeyDecision::Yes);
        assert_eq!(decide(b'y', false), KeyDecision::Yes);
        assert_eq!(decide(b'Y', true), KeyDecision::Yes);
        assert_eq!(decide(b'Y', false), KeyDecision::Yes);
    }

    #[test]
    fn decide_recognises_no_keys() {
        assert_eq!(decide(b'n', true), KeyDecision::No);
        assert_eq!(decide(b'n', false), KeyDecision::No);
        assert_eq!(decide(b'N', true), KeyDecision::No);
        assert_eq!(decide(b'N', false), KeyDecision::No);
    }

    #[test]
    fn decide_treats_enter_as_default() {
        // Bare Enter (LF) and CR both pick the prompt's default; the caller
        // resolves Default to the actual bool, so this branch only signals
        // "user pressed Enter".
        assert_eq!(decide(b'\n', true), KeyDecision::Default);
        assert_eq!(decide(b'\n', false), KeyDecision::Default);
        assert_eq!(decide(b'\r', true), KeyDecision::Default);
        assert_eq!(decide(b'\r', false), KeyDecision::Default);
    }

    #[test]
    fn decide_treats_ctrl_c_and_eot_as_abort() {
        // In raw mode the kernel does not send SIGINT for ^C; we must treat
        // the raw bytes as abort ourselves. Same for ^D (EOF).
        assert_eq!(decide(0x03, true), KeyDecision::Abort);
        assert_eq!(decide(0x03, false), KeyDecision::Abort);
        assert_eq!(decide(0x04, true), KeyDecision::Abort);
        assert_eq!(decide(0x04, false), KeyDecision::Abort);
    }

    #[test]
    fn decide_ignores_irrelevant_keys() {
        // Anything that is not yes/no/Enter/abort should be ignored — the
        // caller loops until it sees a meaningful keystroke. Spot-check a
        // handful of plausible accidental keys.
        for byte in [b'a', b'q', b' ', b'\t', b'0', b'1', 0x1B /* ESC */] {
            assert_eq!(decide(byte, true), KeyDecision::Ignore, "byte {byte:#x}");
            assert_eq!(decide(byte, false), KeyDecision::Ignore, "byte {byte:#x}");
        }
    }
}
