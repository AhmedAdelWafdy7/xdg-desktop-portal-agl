// Copyright 2026 Ahmed Wafdy <ahmedadelwafdy782@gmail.com>
//
// This file is part of xdg-desktop-portal-agl.
//
// xdg-desktop-portal-agl is free software: you can redistribute it and/or
// modify it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 2 of the License, or
// (at your option) any later version.
//
// xdg-desktop-portal-agl is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General
// Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// xdg-desktop-portal-agl. If not, see <https://www.gnu.org/licenses/>.

mod portals;

use crate::portals::{ScreenshotPortal, SettingsPortal};
use zbus::connection;
const INTERFACE_NAME: &str = "org.freedesktop.impl.portal.desktop.agl";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";

// A single-threaded runtime is enough.
#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    tracing::info!("Starting xdg-desktop-portal-agl...");

    let connection = connection::Builder::session()?
        .name(INTERFACE_NAME)?
        .serve_at(PORTAL_PATH, SettingsPortal::new())?
        .serve_at(PORTAL_PATH, ScreenshotPortal::new())?
        .build()
        .await?;

    tracing::info!("xdg-desktop-portal-agl is running.");
    tracing::info!("D-Bus name: {}", INTERFACE_NAME);
    tracing::info!("D-Bus path: {}", PORTAL_PATH);

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Received SIGINT, shutting down.");
        }
        _ = wait_for_signal() => {
            tracing::info!("Received SIGINT, shutting down.");
        }
    }

    drop(connection);
    Ok(())
}

async fn wait_for_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let mut sigterm = signal(SignalKind::terminate()).expect("Failed to set up signal handler");
    sigterm.recv().await;
}
