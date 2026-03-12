use zbus::connection;

const INTERFACE_NAME: &str = "org.freedesktop.impl.portal.desktop.agl";
const PORTAL_PATH:&str = "/org/freedesktop/portal/desktop";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    tracing::info!("Starting xdg-desktop-portal-agl...");
    
    let connection = connection::Builder::session()?
    .name(INTERFACE_NAME)?
    //.serve_at(PORTAL_PATH, iface)
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
    use tokio::signal::unix::{signal, SignalKind};

    let mut sigterm = signal(SignalKind::terminate())
    .expect("Failed to set up signal handler");
    sigterm.recv().await;
}
