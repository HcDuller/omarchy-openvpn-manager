//! omarchy-openvpn-manager: a GTK4/Libadwaita front-end for managing
//! OpenVPN connections through NetworkManager on Omarchy.

mod nm;
mod tray;
mod ui;

fn main() {
    // The tray icon and D-Bus watcher need a tokio runtime; the GTK main
    // loop that relm4 drives is separate and runs on the main thread. We
    // start a background tokio runtime for async NM/tray work and let
    // relm4/gtk own the main thread's event loop.
    let runtime = tokio::runtime::Runtime::new().expect("failed to start tokio runtime");
    let _guard = runtime.enter();

    runtime.spawn(async {
        match tray::spawn().await {
            Ok(handle) => {
                if let Err(err) = watch_and_update_tray(handle).await {
                    eprintln!("tray watcher stopped: {err:#}");
                }
            }
            Err(err) => {
                eprintln!("failed to start tray icon: {err:#}");
            }
        }
    });

    ui::run();
}

/// Keep the tray icon's status in sync with NetworkManager by combining an
/// initial snapshot with live D-Bus state-change notifications.
async fn watch_and_update_tray(handle: tray::TrayHandle) -> anyhow::Result<()> {
    let manager = nm::NetworkManager::new();

    let refresh = |manager: nm::NetworkManager| async move { manager.active_profile().await };

    if let Ok(active) = refresh(manager).await {
        handle
            .set_status(active.is_some(), active.map(|p| p.name))
            .await;
    }

    let mut events = nm::watcher::watch().await?;
    while events.recv().await.is_some() {
        if let Ok(active) = refresh(manager).await {
            handle
                .set_status(active.is_some(), active.map(|p| p.name))
                .await;
        }
    }

    Ok(())
}
