//! FastSSH core: everything that talks to a shell or a remote host.
//!
//! The server and (later) the desktop app both sit on top of this crate, so
//! nothing in here knows about HTTP, websockets or any UI.

pub mod pty;

pub use pty::LocalShell;
