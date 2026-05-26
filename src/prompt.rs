//! Minimal interactive prompts read from stdin. Commands take no flags, so any
//! input a command needs is gathered here.

use std::io::{self, BufRead, Write};

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
pub fn confirm(question: &str, default: bool) -> Result<bool> {
    let hint = if default { "[Y/n]" } else { "[y/N]" };
    print!("{question} {hint} ");
    io::stdout().flush()?;
    let ans = read_line()?.to_lowercase();
    Ok(match ans.as_str() {
        "" => default,
        "y" | "yes" => true,
        "n" | "no" => false,
        _ => default,
    })
}

/// Free-text input; an empty answer takes `default` when one is given.
///
/// Surrounding whitespace is trimmed — suitable for human prose like commit
/// messages. For branch names, use [`input_branch_name`] instead so the
/// caller's keystrokes are preserved verbatim.
pub fn input(prompt: &str, default: Option<&str>) -> Result<String> {
    input_inner(prompt, default, /* preserve */ false)
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
    fn read_trimming_strips_surrounding_whitespace() {
        // The yes/no and numeric picker paths still want trim semantics, so
        // keyword answers like " y " keep working.
        let trimmed: String = read_preserving_from(b"   y   \n").trim().to_string();
        assert_eq!(trimmed, "y");
    }
}
