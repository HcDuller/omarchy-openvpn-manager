//! Thin async wrapper around the `nmcli` command line tool, scoped to
//! OpenVPN connections.

use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// Connection state as reported by NetworkManager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // reserved for future fine-grained status reporting in the UI
pub enum ConnectionState {
    Active,
    Inactive,
    Activating,
    Deactivating,
    Unknown,
}

impl From<&str> for ConnectionState {
    fn from(value: &str) -> Self {
        match value {
            "activated" => ConnectionState::Active,
            "activating" => ConnectionState::Activating,
            "deactivating" => ConnectionState::Deactivating,
            "" => ConnectionState::Inactive,
            _ => ConnectionState::Unknown,
        }
    }
}

/// A single OpenVPN connection profile known to NetworkManager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VpnProfile {
    pub name: String,
    pub uuid: String,
    pub active: bool,
}

const NMCLI: &str = "nmcli";

/// Maximum time to wait for any single `nmcli` invocation before giving up.
/// Prevents the UI from hanging forever if `nmcli`/NetworkManager is stuck
/// (e.g. waiting on a polkit prompt that never appears).
const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);

async fn run(args: &[&str]) -> Result<String> {
    eprintln!("[nm] running: nmcli {}", args.join(" "));

    let child = Command::new(NMCLI)
        .args(args)
        // Explicitly detach stdin so nmcli can never block waiting on
        // interactive input (e.g. a secret prompt) when launched from a
        // desktop entry with no attached terminal.
        .stdin(Stdio::null())
        .output();

    let output = match tokio::time::timeout(COMMAND_TIMEOUT, child).await {
        Ok(result) => {
            result.with_context(|| format!("failed to execute `nmcli {}`", args.join(" ")))?
        }
        Err(_) => {
            bail!(
                "`nmcli {}` timed out after {:?}",
                args.join(" "),
                COMMAND_TIMEOUT
            );
        }
    };

    eprintln!(
        "[nm] finished: nmcli {} -> status={:?}",
        args.join(" "),
        output.status.code()
    );

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("nmcli {} failed: {}", args.join(" "), stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// List all connections of type `vpn` whose vpn-type is openvpn.
pub async fn list_openvpn_profiles() -> Result<Vec<VpnProfile>> {
    // -t: terse/machine-readable, -f: fields, escaped colons handled by nmcli.
    let out = run(&["-t", "-f", "NAME,UUID,TYPE,ACTIVE", "connection", "show"]).await?;

    let mut profiles = Vec::new();
    for line in out.lines() {
        let fields: Vec<&str> = split_nmcli_fields(line);
        if fields.len() < 4 {
            continue;
        }
        let (name, uuid, conn_type, active) = (fields[0], fields[1], fields[2], fields[3]);
        if conn_type != "vpn" {
            continue;
        }
        profiles.push(VpnProfile {
            name: name.to_string(),
            uuid: uuid.to_string(),
            active: active == "yes",
        });
    }

    Ok(profiles)
}

/// Return the currently active OpenVPN profile, if any.
pub async fn active_openvpn_profile() -> Result<Option<VpnProfile>> {
    let profiles = list_openvpn_profiles().await?;
    Ok(profiles.into_iter().find(|p| p.active))
}

/// Import an `.ovpn` file as a NetworkManager connection.
///
/// Returns the connection name nmcli reports for the new profile.
pub async fn import_ovpn(path: &Path) -> Result<String> {
    let path_str = path.to_str().context("ovpn file path is not valid UTF-8")?;

    let out = run(&["connection", "import", "type", "openvpn", "file", path_str]).await?;

    // nmcli prints e.g.: `Connection 'my-vpn' (uuid) successfully added.`
    let name = out
        .split('\'')
        .nth(1)
        .map(str::to_string)
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("imported-vpn")
                .to_string()
        });

    Ok(name)
}

pub async fn connection_up(name: &str) -> Result<()> {
    // --ask tells nmcli to allow interactive secret retrieval (sets
    // AllowInteraction=TRUE on the underlying D-Bus ActivateConnection
    // call). Without it, NetworkManager never queries registered secret
    // agents at all (ours included) and nmcli fails immediately with
    // "... cannot ask without --ask option" whenever a secret like the VPN
    // password isn't already embedded in the connection. Our registered
    // secret agent answers the resulting GetSecrets request, so stdin
    // being null (see `run`) is fine - nmcli's own terminal-prompt
    // fallback is never reached as long as our agent supplies the secret.
    run(&["connection", "up", name, "--ask"]).await?;
    Ok(())
}

pub async fn connection_down(name: &str) -> Result<()> {
    run(&["connection", "down", name]).await?;
    Ok(())
}

pub async fn connection_delete(name: &str) -> Result<()> {
    run(&["connection", "delete", name]).await?;
    Ok(())
}

/// Details parsed from a connection's `vpn.data` property, used to decide
/// whether an import-review/edit dialog is needed and to pre-fill it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VpnConnectionDetails {
    /// Whether this connection's auth method requires a username/password
    /// (NetworkManager-openvpn's `connection-type` is `password` or
    /// `password-tls`), as opposed to certificate-only or static-key auth.
    pub needs_auth: bool,
    /// Username embedded in the connection, if any (rarely present from a
    /// plain `.ovpn` import, since `auth-user-pass` files don't usually
    /// carry the username itself).
    pub username: Option<String>,
}

/// Parse the `vpn.data` map property of a connection to determine whether
/// it needs username/password credentials, and any already-known username.
///
/// Best-effort parser: `vpn.data` is a comma-separated `key = value` map as
/// printed by `nmcli -g`. This has not been validated against every
/// NetworkManager-openvpn plugin version; if key names differ, callers will
/// simply see `needs_auth: false` and no pre-filled username rather than a
/// hard failure.
pub async fn get_connection_details(name: &str) -> Result<VpnConnectionDetails> {
    let out = run(&["-g", "vpn.data", "connection", "show", name]).await?;
    let data = parse_vpn_data(&out);

    let needs_auth = data
        .get("connection-type")
        .map(|v| v == "password" || v == "password-tls")
        .unwrap_or(false);

    Ok(VpnConnectionDetails {
        needs_auth,
        username: data.get("username").cloned(),
    })
}

/// Parses nmcli's `key1 = value1, key2 = value2` map property format into a
/// lookup table. Does not attempt to handle escaped commas within values.
fn parse_vpn_data(raw: &str) -> HashMap<String, String> {
    raw.trim()
        .split(", ")
        .filter_map(|pair| {
            let (key, value) = pair.split_once(" = ")?;
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

/// Sets the `username` sub-key of a connection's `vpn.data` map, without
/// disturbing other keys already present.
pub async fn set_vpn_username(name: &str, username: &str) -> Result<()> {
    run(&[
        "connection",
        "modify",
        name,
        "+vpn.data",
        &format!("username={username}"),
    ])
    .await?;
    Ok(())
}

/// Marks the VPN password as agent-owned (`password-flags = 1`), so
/// NetworkManager asks a running Secret Agent for it at connect time
/// instead of expecting it embedded in the connection or prompting
/// interactively (which requires a TTY we don't have from a GUI app).
pub async fn mark_password_agent_owned(name: &str) -> Result<()> {
    run(&[
        "connection",
        "modify",
        name,
        "+vpn.data",
        "password-flags=1",
    ])
    .await?;
    Ok(())
}

/// Split a line of `nmcli -t` output on unescaped `:` separators.
fn split_nmcli_fields(line: &str) -> Vec<&str> {
    // nmcli escapes literal colons in field values as `\:`. For the fields we
    // consume (NAME, UUID, TYPE, ACTIVE) this naive split is sufficient for
    // the common case; a fully correct unescaper would walk char-by-char.
    line.split(':').collect()
}
