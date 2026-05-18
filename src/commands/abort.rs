//! `gt abort` — undo an in-progress operation and restore the prior state.

use crate::error::Result;
use crate::rebase;

pub fn run() -> Result<()> {
    rebase::abort()
}
