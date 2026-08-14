//! Implements a NetworkManager Secret Agent
//! (`org.freedesktop.NetworkManager.SecretAgent`) so NM can ask us for VPN
//! passwords at connect time, instead of requiring `--ask` (no TTY available
//! from a GUI app) or leaving the password unset (which is what caused
//! `nmcli connection up` to fail with "password ... not given").
//!
//! Passwords are looked up from/stored to the OS keyring via
//! [`crate::secrets::keyring`], never written to our own files or to
//! NetworkManager's connection files in plaintext.

use super::keyring;
use std::collections::HashMap;
use zbus::interface;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

const AGENT_IDENTIFIER: &str = "org.omarchy.OpenvpnManager";
const AGENT_PATH: &str = "/org/freedesktop/NetworkManager/SecretAgent";

/// A NetworkManager connection's settings, as passed to secret agent
/// methods: a map of setting name (e.g. "vpn", "connection") to a map of
/// property name to value.
type ConnectionSettings = HashMap<String, HashMap<String, OwnedValue>>;

pub struct VpnSecretAgent;

#[interface(name = "org.freedesktop.NetworkManager.SecretAgent")]
impl VpnSecretAgent {
    /// Called by NetworkManager when it needs a secret (e.g. the VPN
    /// password) to complete a connection.
    async fn get_secrets(
        &self,
        connection: ConnectionSettings,
        _connection_path: ObjectPath<'_>,
        setting_name: String,
        _hints: Vec<String>,
        _flags: u32,
    ) -> zbus::fdo::Result<ConnectionSettings> {
        let uuid = connection
            .get("connection")
            .and_then(|c| c.get("uuid"))
            .and_then(|v| v.downcast_ref::<&str>().ok())
            .map(str::to_string);

        let Some(uuid) = uuid else {
            eprintln!("[secret-agent] GetSecrets: connection had no uuid, cannot look up secret");
            return Err(zbus::fdo::Error::Failed(
                "connection has no uuid".to_string(),
            ));
        };

        eprintln!("[secret-agent] GetSecrets for uuid={uuid} setting={setting_name}");

        let password = keyring::lookup_password(&uuid).await.map_err(|err| {
            eprintln!("[secret-agent] keyring lookup failed: {err:#}");
            zbus::fdo::Error::Failed(format!("keyring lookup failed: {err}"))
        })?;

        let Some(password) = password else {
            eprintln!("[secret-agent] no stored password for uuid={uuid}");
            return Err(zbus::fdo::Error::Failed(format!(
                "no stored password for connection {uuid}"
            )));
        };

        let mut secrets = HashMap::new();
        let mut vpn_secrets = HashMap::new();
        vpn_secrets.insert(
            "password".to_string(),
            OwnedValue::try_from(Value::from(password))
                .map_err(|err| zbus::fdo::Error::Failed(err.to_string()))?,
        );
        secrets.insert(setting_name, vpn_secrets);

        Ok(secrets)
    }

    /// Called by NetworkManager when it's told to persist secrets (e.g.
    /// after a successful connect with newly-provided credentials).
    async fn save_secrets(&self, connection: ConnectionSettings, _connection_path: ObjectPath<'_>) {
        let uuid = connection
            .get("connection")
            .and_then(|c| c.get("uuid"))
            .and_then(|v| v.downcast_ref::<&str>().ok())
            .map(str::to_string);

        let Some(uuid) = uuid else {
            eprintln!("[secret-agent] SaveSecrets: connection had no uuid");
            return;
        };

        let password = connection
            .get("vpn")
            .and_then(|vpn| vpn.get("secrets"))
            .and_then(|v| v.downcast_ref::<&str>().ok())
            .map(str::to_string);

        if let Some(password) = password {
            let label = connection
                .get("connection")
                .and_then(|c| c.get("id"))
                .and_then(|v| v.downcast_ref::<&str>().ok())
                .unwrap_or("VPN connection")
                .to_string();

            if let Err(err) = keyring::store_password(&uuid, &label, &password).await {
                eprintln!("[secret-agent] failed to store secret in keyring: {err:#}");
            }
        }
    }

    /// Called by NetworkManager to cancel an in-flight `GetSecrets` request.
    /// We don't do any long-running interactive prompting, so there's
    /// nothing to cancel.
    async fn cancel_get_secrets(&self, _connection_path: ObjectPath<'_>, _setting_name: String) {}

    /// Called by NetworkManager when the user asks to forget a connection's
    /// secrets (e.g. removing a saved password).
    async fn delete_secrets(
        &self,
        connection: ConnectionSettings,
        _connection_path: ObjectPath<'_>,
    ) {
        let uuid = connection
            .get("connection")
            .and_then(|c| c.get("uuid"))
            .and_then(|v| v.downcast_ref::<&str>().ok())
            .map(str::to_string);

        if let Some(uuid) = uuid {
            if let Err(err) = keyring::delete_password(&uuid).await {
                eprintln!("[secret-agent] failed to delete secret from keyring: {err:#}");
            }
        }
    }
}

/// Registers our secret agent object on the system bus and tells
/// NetworkManager to start using it via `AgentManager.Register`.
///
/// Keeps `conn` alive for as long as the agent should remain registered
/// (NM drops agents when their D-Bus connection closes).
pub async fn register() -> anyhow::Result<zbus::Connection> {
    let conn = zbus::Connection::system().await?;
    conn.object_server().at(AGENT_PATH, VpnSecretAgent).await?;

    let proxy = zbus::Proxy::new(
        &conn,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager/AgentManager",
        "org.freedesktop.NetworkManager.AgentManager",
    )
    .await?;

    proxy.call_method("Register", &(AGENT_IDENTIFIER,)).await?;

    eprintln!("[secret-agent] registered as {AGENT_IDENTIFIER}");

    Ok(conn)
}
