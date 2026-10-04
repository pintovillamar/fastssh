//! Stand-in for platforms where the local shell is not built yet (Windows).
//!
//! It has the same shape as the real [`LocalShell`], so callers need no
//! platform checks of their own, but it can never be started.

use anyhow::{Result, bail};
use tokio::sync::mpsc;

/// A local shell on a platform that has none yet. The type has no values:
/// [`LocalShell::spawn`] always fails, so the other methods are unreachable.
pub enum LocalShell {}

impl LocalShell {
    /// Whether this build can open a shell on the machine it runs on.
    pub const SUPPORTED: bool = false;

    pub fn spawn(_cols: u16, _rows: u16) -> Result<(Self, mpsc::Receiver<Vec<u8>>)> {
        bail!("the local shell is not available on this platform yet")
    }

    pub fn write(&self, _bytes: Vec<u8>) {
        match *self {}
    }

    pub fn resize(&self, _cols: u16, _rows: u16) -> Result<()> {
        match *self {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cannot_be_started() {
        assert!(LocalShell::spawn(80, 24).is_err());
    }
}
