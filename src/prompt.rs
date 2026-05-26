//! Minimal interactive prompts read from stdin. Commands take no flags, so any
//! input a command needs is gathered here.

use std::io::{self, BufRead, Write};

use crate::error::{GtError, Result};
use crate::tree::TreeLine;

fn read_line() -> Result<String> {
    let mut s = String::new();
    let n = io::stdin().lock().read_line(&mut s)?;
    if n == 0 {
        // EOF with no answer — treat as a declined prompt.
        return Err(GtError::Aborted);
    }
    Ok(s.trim().to_string())
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
pub fn input(prompt: &str, default: Option<&str>) -> Result<String> {
    match default {
        Some(d) => print!("{prompt} [{d}]: "),
        None => print!("{prompt}: "),
    }
    io::stdout().flush()?;
    let ans = read_line()?;
    if ans.is_empty() {
        match default {
            Some(d) => Ok(d.to_string()),
            None => Err(GtError::Usage("a value is required".into())),
        }
    } else {
        Ok(ans)
    }
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
