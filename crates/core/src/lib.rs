//! FastSSH core: everything that talks to a shell or a remote host.
//!
//! The server and (later) the desktop app both sit on top of this crate, so
//! nothing in here knows about HTTP, websockets or any UI.

pub mod pty;
pub mod ssh;
pub mod store;

pub use pty::LocalShell;
pub use ssh::SshShell;
pub use store::Store;

/// Any interactive shell, local or remote.
pub enum Shell {
    Local(LocalShell),
    Ssh(SshShell),
}

impl Shell {
    /// Sends keystrokes (or pasted text) to the shell.
    pub async fn write(&self, bytes: Vec<u8>) {
        match self {
            Shell::Local(shell) => shell.write(bytes),
            Shell::Ssh(shell) => shell.write(bytes).await,
        }
    }

    pub async fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        match self {
            Shell::Local(shell) => shell.resize(cols, rows),
            Shell::Ssh(shell) => {
                shell.resize(cols, rows).await;
                Ok(())
            }
        }
    }
}
