//! FastSSH as a desktop app.
//!
//! The app runs the same server as the `fastssh` command, privately on a
//! free localhost port, and shows its interface in a native window. Nothing
//! is exposed to the network, and the local shell is this computer's.

// On Windows, don't open a console window next to the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "fastssh=info".into()),
        )
        .init();

    tauri::Builder::default()
        .setup(|app| {
            let options = fastssh::Options {
                data_dir: app.path().app_data_dir()?,
                local_only: true,
                public_url: None,
                allow_signup: false,
                local_shell: true,
                google: None,
                desktop: true,
            };
            // The server needs Tauri's async runtime around it to start.
            let address = tauri::async_runtime::block_on(async { fastssh::spawn_local(options) })?;
            tracing::info!("interface served at http://{address}");

            let url = format!("http://{address}").parse()?;
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("FastSSH")
                .inner_size(1100.0, 720.0)
                .min_inner_size(420.0, 320.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running FastSSH");
}
