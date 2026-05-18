//! `gt continue` — resume an operation after conflicts have been resolved.

use crate::error::Result;
use crate::rebase;

pub fn run() -> Result<()> {
    rebase::resume()
}
