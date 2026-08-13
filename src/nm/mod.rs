//! NetworkManager integration layer.
//!
//! This module wraps `nmcli` for managing OpenVPN connection profiles and
//! exposes a small, testable API used by the GUI and tray layers. It also
//! provides a D-Bus based watcher (see `watcher`) for live connection state
//! updates instead of polling.

mod cli;
pub mod watcher;

#[allow(unused_imports)]
// ConnectionState is reserved for future fine-grained status reporting
pub use cli::{ConnectionState, VpnProfile};

use anyhow::Result;
use std::path::Path;

/// Handle for interacting with NetworkManager's OpenVPN connections.
#[derive(Debug, Default, Clone, Copy)]
pub struct NetworkManager;

impl NetworkManager {
    pub fn new() -> Self {
        Self
    }

    /// List all VPN connections of type openvpn known to NetworkManager.
    pub async fn list_profiles(&self) -> Result<Vec<VpnProfile>> {
        cli::list_openvpn_profiles().await
    }

    /// Import a `.ovpn` file as a new NetworkManager connection profile.
    ///
    /// Returns the connection name NetworkManager assigned to the import.
    pub async fn import_profile(&self, ovpn_path: &Path) -> Result<String> {
        cli::import_ovpn(ovpn_path).await
    }

    /// Bring the given connection up (connect).
    pub async fn connect(&self, name: &str) -> Result<()> {
        cli::connection_up(name).await
    }

    /// Bring the given connection down (disconnect).
    pub async fn disconnect(&self, name: &str) -> Result<()> {
        cli::connection_down(name).await
    }

    /// Permanently delete a connection profile.
    pub async fn delete_profile(&self, name: &str) -> Result<()> {
        cli::connection_delete(name).await
    }

    /// Get the currently active VPN profile, if any.
    pub async fn active_profile(&self) -> Result<Option<VpnProfile>> {
        cli::active_openvpn_profile().await
    }
}
