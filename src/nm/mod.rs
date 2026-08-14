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
pub use cli::{ConnectionState, VpnConnectionDetails, VpnProfile};

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

    /// Fetch parsed `vpn.data` details for a connection (whether it needs
    /// username/password credentials, and any already-known username).
    pub async fn connection_details(&self, name: &str) -> Result<VpnConnectionDetails> {
        cli::get_connection_details(name).await
    }

    /// Set the username on a connection's VPN settings.
    pub async fn set_username(&self, name: &str, username: &str) -> Result<()> {
        cli::set_vpn_username(name, username).await
    }

    /// Set the remote server address.
    pub async fn set_remote(&self, name: &str, remote: &str) -> Result<()> {
        cli::set_vpn_remote(name, remote).await
    }

    /// Set the remote server port.
    pub async fn set_port(&self, name: &str, port: &str) -> Result<()> {
        cli::set_vpn_port(name, port).await
    }

    /// Set the transport protocol (TCP if `true`, UDP if `false`).
    pub async fn set_protocol_tcp(&self, name: &str, is_tcp: bool) -> Result<()> {
        cli::set_vpn_protocol_tcp(name, is_tcp).await
    }

    /// Set the data cipher.
    pub async fn set_cipher(&self, name: &str, cipher: &str) -> Result<()> {
        cli::set_vpn_cipher(name, cipher).await
    }

    /// Mark a connection's VPN password as agent-owned, so our registered
    /// Secret Agent is asked for it at connect time.
    pub async fn mark_password_agent_owned(&self, name: &str) -> Result<()> {
        cli::mark_password_agent_owned(name).await
    }
}
