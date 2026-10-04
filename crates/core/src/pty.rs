//! A local shell running inside a pseudo-terminal.

use std::io::{Read, Write};
use std::sync::mpsc as std_mpsc;
use std::thread;

use anyhow::{Context, Result};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::mpsc;

/// How many output chunks may queue up before the reader thread waits for the
/// consumer. This is what stops `yes` from filling memory on a slow link.
const OUTPUT_BACKLOG: usize = 64;

/// The user's login shell attached to a pty.
///
/// The pty API is blocking, so reads and writes each get their own OS thread
/// and talk to async code through channels. Dropping the value ends the shell.
pub struct LocalShell {
    master: Box<dyn MasterPty + Send>,
    input: std_mpsc::Sender<Vec<u8>>,
    child: Option<Box<dyn Child + Send + Sync>>,
}

impl LocalShell {
    /// Starts the shell and returns it together with its output stream.
    /// The stream ends when the shell exits.
    pub fn spawn(cols: u16, rows: u16) -> Result<(Self, mpsc::Receiver<Vec<u8>>)> {
        let pair = native_pty_system()
            .openpty(size(cols, rows))
            .context("opening a pty")?;

        let mut cmd = CommandBuilder::new_default_prog();
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        if let Some(home) = std::env::var_os("HOME") {
            cmd.cwd(home);
        }
        let child = pair.slave.spawn_command(cmd).context("starting the shell")?;
        // The child holds its own copy of the slave end. Keeping ours open
        // would stop the reader from ever seeing end-of-file.
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().context("pty reader")?;
        let mut writer = pair.master.take_writer().context("pty writer")?;

        let (output_tx, output_rx) = mpsc::channel(OUTPUT_BACKLOG);
        thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if output_tx.blocking_send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        let (input_tx, input_rx) = std_mpsc::channel::<Vec<u8>>();
        thread::spawn(move || {
            for bytes in input_rx {
                if writer.write_all(&bytes).is_err() {
                    break;
                }
            }
        });

        let shell = Self {
            master: pair.master,
            input: input_tx,
            child: Some(child),
        };
        Ok((shell, output_rx))
    }

    /// Sends keystrokes (or pasted text) to the shell.
    pub fn write(&self, bytes: Vec<u8>) {
        // A send error means the shell already exited; the closed output
        // stream is how callers find out.
        let _ = self.input.send(bytes);
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master.resize(size(cols, rows)).context("resizing pty")
    }
}

impl Drop for LocalShell {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // Reap on a separate thread so dropping never blocks async code.
            thread::spawn(move || {
                let _ = child.kill();
                let _ = child.wait();
            });
        }
    }
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        cols: cols.max(1),
        rows: rows.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}
