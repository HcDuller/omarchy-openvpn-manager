//! System tray icon (StatusNotifierItem via `ksni`) showing VPN connection
//! status with a quick connect/disconnect toggle for the active profile.

use crate::nm::NetworkManager;
use ksni::{menu::StandardItem, Handle, MenuItem, Tray, TrayMethods};
use std::sync::{Arc, Mutex};

/// Data displayed by the tray icon, updated whenever connection state
/// changes.
#[derive(Default, Clone)]
struct TrayState {
    connected: bool,
    active_profile: Option<String>,
}

struct VpnTray {
    state: Arc<Mutex<TrayState>>,
}

impl Tray for VpnTray {
    fn id(&self) -> String {
        env!("CARGO_PKG_NAME").into()
    }

    fn icon_name(&self) -> String {
        // Symbolic icon names are used here (rather than the unsuffixed
        // variants) because Adwaita, the icon theme Omarchy/GTK4 apps
        // assume as a baseline, only ships `-symbolic` VPN status icons.
        // Symbolic icons also auto-adapt to light/dark tray backgrounds.
        if self.state.lock().unwrap().connected {
            "network-vpn-symbolic".into()
        } else {
            "network-vpn-disconnected-symbolic".into()
        }
    }

    fn title(&self) -> String {
        "OpenVPN Manager".into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let state = self.state.lock().unwrap();
        let description = match (&state.active_profile, state.connected) {
            (Some(name), true) => format!("Connected: {name}"),
            (Some(name), false) => format!("Disconnected ({name} available)"),
            (None, _) => "No VPN profiles configured".to_string(),
        };
        ksni::ToolTip {
            title: "OpenVPN Manager".into(),
            description,
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let state = self.state.lock().unwrap().clone();

        let toggle_label = if state.connected {
            "Disconnect".to_string()
        } else {
            "Connect".to_string()
        };
        let toggle_profile = state.active_profile.clone();
        let toggle_connected = state.connected;

        vec![
            StandardItem {
                label: toggle_label,
                enabled: toggle_profile.is_some(),
                activate: Box::new(move |_this: &mut Self| {
                    let profile = toggle_profile.clone();
                    let connected = toggle_connected;
                    tokio::spawn(async move {
                        if let Some(name) = profile {
                            let nm = NetworkManager::new();
                            let result = if connected {
                                nm.disconnect(&name).await
                            } else {
                                nm.connect(&name).await
                            };
                            if let Err(err) = result {
                                eprintln!("tray toggle failed: {err:#}");
                            }
                        }
                    });
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Open OpenVPN Manager".into(),
                activate: Box::new(|_this: &mut Self| {
                    crate::ui::request_show_window();
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|_this: &mut Self| {
                    crate::ui::request_quit();
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Handle to the running tray service, used to push connection status
/// updates from elsewhere in the application (e.g. the D-Bus watcher).
pub struct TrayHandle {
    handle: Handle<VpnTray>,
    state: Arc<Mutex<TrayState>>,
}

impl TrayHandle {
    /// Update the connection status shown by the tray icon and notify the
    /// host to refresh (icon, tooltip, menu).
    pub async fn set_status(&self, connected: bool, active_profile: Option<String>) {
        {
            let mut state = self.state.lock().unwrap();
            state.connected = connected;
            state.active_profile = active_profile;
        }
        // Trigger a PropertiesChanged notification; the closure itself does
        // not need to mutate anything since we already updated the shared
        // state above.
        let _ = self.handle.update(|_tray: &mut VpnTray| {}).await;
    }
}

/// Spawn the tray icon in the background. Returns a handle used to push
/// status updates.
pub async fn spawn() -> anyhow::Result<TrayHandle> {
    let state = Arc::new(Mutex::new(TrayState::default()));
    let tray = VpnTray {
        state: state.clone(),
    };
    let handle = tray.spawn().await?;
    Ok(TrayHandle { handle, state })
}
