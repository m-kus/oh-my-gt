//! Minimal interactive prompts read from stdin. Commands take no flags, so any
//! input a command needs is gathered here.

use std::io::{self, BufRead, Write};

use crate::error::{GtError, Result};

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
