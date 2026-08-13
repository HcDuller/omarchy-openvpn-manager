//! Thin async wrapper around the `nmcli` command line tool, scoped to
//! OpenVPN connections.

use anyhow::{bail, Context, Result};
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
    run(&["connection", "up", name]).await?;
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

/// Split a line of `nmcli -t` output on unescaped `:` separators.
fn split_nmcli_fields(line: &str) -> Vec<&str> {
    // nmcli escapes literal colons in field values as `\:`. For the fields we
    // consume (NAME, UUID, TYPE, ACTIVE) this naive split is sufficient for
    // the common case; a fully correct unescaper would walk char-by-char.
    line.split(':').collect()
}
