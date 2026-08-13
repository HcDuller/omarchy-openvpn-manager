//! Watches NetworkManager over D-Bus for VPN connection state changes,
//! avoiding the need to poll `nmcli` on a timer.

use anyhow::Result;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use zbus::{proxy, Connection};

/// Simplified state broadcast to listeners whenever NetworkManager reports
/// a change relevant to VPN connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VpnEvent {
    /// A connection activated, deactivated, or otherwise changed state.
    StateChanged,
}

#[proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait NetworkManagerProxy {
    #[zbus(signal)]
    fn state_changed(&self, state: u32) -> zbus::Result<()>;
}

/// Subscribe to NetworkManager D-Bus signals and forward simplified events
/// over the returned channel. The task runs until the connection is dropped.
pub async fn watch() -> Result<mpsc::UnboundedReceiver<VpnEvent>> {
    let (tx, rx) = mpsc::unbounded_channel();

    let connection = Connection::system().await?;
    let proxy = NetworkManagerProxyProxy::new(&connection).await?;

    let mut state_stream = proxy.receive_state_changed().await?;
    let tx_state = tx.clone();
    tokio::spawn(async move {
        while state_stream.next().await.is_some() {
            if tx_state.send(VpnEvent::StateChanged).is_err() {
                break;
            }
        }
    });

    Ok(rx)
}
