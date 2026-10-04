//! FastSSH core: everything that talks to a shell or a remote host.
//!
//! The server and (later) the desktop app both sit on top of this crate, so
//! nothing in here knows about HTTP, websockets or any UI.

pub mod pty;
pub mod ssh;
pub mod store;
pub mod vault;

pub use pty::LocalShell;
pub use ssh::SshShell;
pub use store::Store;

use serde::{Deserialize, Serialize};
use vault::VaultKey;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// The secret part of a saved connection. Lives encrypted in the database and
/// in the clear only while a session is being opened or the form is saved.
#[derive(Default, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Secrets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_passphrase: Option<String>,
}

impl Secrets {
    pub fn is_empty(&self) -> bool {
        self.password.is_none() && self.private_key.is_none() && self.key_passphrase.is_none()
    }

    /// `None` when there is nothing worth storing.
    pub fn seal(&self, key: &VaultKey) -> Option<Vec<u8>> {
        if self.is_empty() {
            return None;
        }
        let mut json = serde_json::to_vec(self).expect("secrets serialize");
        let sealed = key.encrypt(&json);
        json.zeroize();
        Some(sealed)
    }

    /// Fails if the blob was not written with this key.
    pub fn open(sealed: Option<&[u8]>, key: &VaultKey) -> anyhow::Result<Self> {
        let Some(sealed) = sealed else {
            return Ok(Self::default());
        };
        let mut json = key
            .decrypt(sealed)
            .ok_or_else(|| anyhow::anyhow!("saved secrets could not be decrypted"))?;
        let secrets = serde_json::from_slice(&json);
        json.zeroize();
        Ok(secrets?)
    }
}

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
