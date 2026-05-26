//! Minimal ANSI styling for human-facing output.
//!
//! Captured output stays plain: colors are only emitted for terminal streams,
//! and the presence of `NO_COLOR` disables styling entirely.

use std::fmt;
use std::io::{self, IsTerminal};

#[derive(Clone, Copy)]
pub(crate) struct OutputStyle {
    color: bool,
}

#[derive(Clone, Copy)]
enum Stream {
    Stdout,
    Stderr,
}

const ERROR: &str = "1;31";
const WARNING: &str = "1;33";
const SUCCESS: &str = "1;32";
const STATUS: &str = "36";
const BRANCH: &str = "1;36";
const GLYPH: &str = "90";
const RESTACK: &str = "1;33";
const RESET: &str = "\x1b[0m";

impl OutputStyle {
    pub(crate) fn stdout() -> Self {
        Self::for_stream(Stream::Stdout)
    }

    pub(crate) fn stderr() -> Self {
        Self::for_stream(Stream::Stderr)
    }

    fn for_stream(stream: Stream) -> Self {
        Self::from_flags(is_terminal(stream), std::env::var_os("NO_COLOR").is_some())
    }

    fn from_flags(is_terminal: bool, no_color: bool) -> Self {
        Self {
            color: is_terminal && !no_color,
        }
    }

    pub(crate) fn error<T: fmt::Display>(&self, value: T) -> Styled<T> {
        self.paint(value, ERROR)
    }

    pub(crate) fn warning<T: fmt::Display>(&self, value: T) -> Styled<T> {
        self.paint(value, WARNING)
    }

    pub(crate) fn success<T: fmt::Display>(&self, value: T) -> Styled<T> {
        self.paint(value, SUCCESS)
    }

    pub(crate) fn status<T: fmt::Display>(&self, value: T) -> Styled<T> {
        self.paint(value, STATUS)
    }

    pub(crate) fn branch<T: fmt::Display>(&self, value: T) -> Styled<T> {
        self.paint(value, BRANCH)
    }

    pub(crate) fn glyph<T: fmt::Display>(&self, value: T) -> Styled<T> {
        self.paint(value, GLYPH)
    }

    pub(crate) fn restack_marker<T: fmt::Display>(&self, value: T) -> Styled<T> {
        self.paint(value, RESTACK)
    }

    fn paint<T: fmt::Display>(&self, value: T, code: &'static str) -> Styled<T> {
        Styled {
            value,
            code,
            enabled: self.color,
        }
    }
}

pub(crate) struct Styled<T> {
    value: T,
    code: &'static str,
    enabled: bool,
}

impl<T: fmt::Display> fmt::Display for Styled<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.enabled {
            write!(f, "\x1b[{}m{}{}", self.code, self.value, RESET)
        } else {
            self.value.fmt(f)
        }
    }
}

fn is_terminal(stream: Stream) -> bool {
    match stream {
        Stream::Stdout => io::stdout().is_terminal(),
        Stream::Stderr => io::stderr().is_terminal(),
    }
}

#[cfg(test)]
mod tests {
    use super::OutputStyle;

    #[test]
    fn captured_output_is_plain() {
        let style = OutputStyle::from_flags(false, false);

        assert_eq!(style.error("error:").to_string(), "error:");
        assert_eq!(style.branch("feature").to_string(), "feature");
    }

    #[test]
    fn no_color_disables_terminal_color() {
        let style = OutputStyle::from_flags(true, true);

        assert_eq!(style.warning("warning:").to_string(), "warning:");
        assert_eq!(style.success("created").to_string(), "created");
    }

    #[test]
    fn terminal_without_no_color_uses_ansi() {
        let style = OutputStyle::from_flags(true, false);

        assert_eq!(style.error("error:").to_string(), "\x1b[1;31merror:\x1b[0m");
        assert_eq!(
            style.restack_marker("(needs restack)").to_string(),
            "\x1b[1;33m(needs restack)\x1b[0m"
        );
    }
}
