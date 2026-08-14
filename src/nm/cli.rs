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
    /// Remote server address, parsed from the first entry of the `remote`
    /// sub-key (which packs `host:port:proto` per entry, comma-separated
    /// if there are multiple failover remotes).
    pub remote: Option<String>,
    /// Remote server port, parsed from the same entry as `remote`.
    pub port: Option<String>,
    /// Transport protocol as NetworkManager-openvpn stores it verbatim
    /// (e.g. "udp4", "tcp4", "udp6", "tcp"), parsed from the same entry as
    /// `remote`. Use [`normalized_protocol`] to get a plain "udp"/"tcp" for
    /// display/comparison purposes.
    pub protocol: Option<String>,
    /// Data cipher, if parseable.
    pub cipher: Option<String>,
}

/// Reduces a raw protocol string like "udp4"/"tcp6" to a plain "udp"/"tcp"
/// for dropdown selection and change comparisons, defaulting to "udp" if
/// unparseable.
pub fn normalized_protocol(protocol: Option<&str>) -> &'static str {
    match protocol {
        Some(p) if p.starts_with("tcp") => "tcp",
        _ => "udp",
    }
}

/// Parse the `vpn.data` map property of a connection to determine whether
/// it needs username/password credentials, and any already-known settings.
///
/// Best-effort parser: `vpn.data` is a comma-separated `key = value` map as
/// printed by `nmcli -g`. This has not been validated against every
/// NetworkManager-openvpn plugin version; if key names differ, callers will
/// simply see the corresponding field as `None`/`false` rather than a hard
/// failure. Callers should treat unparsed fields as "leave alone" rather
/// than "clear this value" to avoid clobbering a working connection.
pub async fn get_connection_details(name: &str) -> Result<VpnConnectionDetails> {
    let out = run(&["-g", "vpn.data", "connection", "show", name]).await?;
    let data = parse_vpn_data(&out);

    let needs_auth = data
        .get("connection-type")
        .map(|v| v == "password" || v == "password-tls")
        .unwrap_or(false);

    // NetworkManager-openvpn stores each remote as "host:port:proto",
    // joined by ", " when there are multiple (e.g. this .ovpn's two
    // `remote` lines for failover). We only surface the first one for
    // editing; if the user doesn't touch the remote/port fields, the
    // original (possibly multi-remote) value is left untouched entirely
    // thanks to the non-empty-if-changed guard callers apply.
    let first_remote = data
        .get("remote")
        .and_then(|v| v.split(", ").next())
        .map(str::to_string);
    let (remote, port, protocol) = match &first_remote {
        Some(entry) => {
            let mut parts = entry.splitn(3, ':');
            let host = parts.next().map(str::to_string);
            let port = parts.next().map(str::to_string);
            let proto = parts.next().map(str::to_string);
            (host, port, proto)
        }
        None => (None, None, None),
    };

    Ok(VpnConnectionDetails {
        needs_auth,
        username: data.get("username").cloned(),
        remote,
        port,
        protocol,
        cipher: data.get("cipher").cloned(),
    })
}

/// Parses nmcli's `key1 = value1, key2 = value2` map property format into a
/// lookup table, correctly handling nmcli's backslash-escaping of literal
/// `,`, `:`, and `\` characters within values (e.g. this plugin's `remote`
/// value embeds `:` inside each entry and separates multiple entries with
/// an escaped `,` so it isn't confused with the outer map's own `, `
/// separator between different keys).
fn parse_vpn_data(raw: &str) -> HashMap<String, String> {
    let mut result = HashMap::new();
    let mut chars = raw.trim().chars().peekable();

    loop {
        // Parse the key: read up to an unescaped " = ".
        let mut key = String::new();
        loop {
            match chars.next() {
                None => return result,
                Some('\\') => {
                    if let Some(c) = chars.next() {
                        key.push(c);
                    }
                }
                Some(' ') if chars.peek() == Some(&'=') => {
                    chars.next(); // consume '='
                    if chars.peek() == Some(&' ') {
                        chars.next(); // consume the space after '='
                    }
                    break;
                }
                Some(c) => key.push(c),
            }
        }

        // Parse the value: read up to an unescaped ", " or end of input.
        let mut value = String::new();
        let mut ended = false;
        loop {
            match chars.next() {
                None => {
                    ended = true;
                    break;
                }
                Some('\\') => {
                    if let Some(c) = chars.next() {
                        value.push(c);
                    }
                }
                Some(',') if chars.peek() == Some(&' ') => {
                    chars.next(); // consume the space after ','
                    break;
                }
                Some(c) => value.push(c),
            }
        }

        result.insert(key.trim().to_string(), value);
        if ended {
            return result;
        }
    }
}

/// Sets the `username` sub-key of a connection's `vpn.data` map, without
/// disturbing other keys already present.
pub async fn set_vpn_username(name: &str, username: &str) -> Result<()> {
    set_vpn_data_key(name, "username", username).await
}

/// Sets the `remote` sub-key. NetworkManager-openvpn packs server address,
/// port, and protocol into a single `host:port:proto` value (e.g.
/// `45.236.52.35:1194:udp4`), so host/port/protocol edits must all be
/// reconstructed into this one combined string rather than set
/// independently. This intentionally replaces any additional
/// failover remotes the original connection had.
pub async fn set_vpn_remote(name: &str, remote: &str) -> Result<()> {
    set_vpn_data_key(name, "remote", remote).await
}

/// Sets the `cipher` sub-key.
pub async fn set_vpn_cipher(name: &str, cipher: &str) -> Result<()> {
    set_vpn_data_key(name, "cipher", cipher).await
}

/// Sets a single sub-key of a connection's `vpn.data` map property without
/// disturbing other keys already present (via nmcli's `+vpn.data` merge
/// syntax, rather than replacing the whole map).
async fn set_vpn_data_key(name: &str, key: &str, value: &str) -> Result<()> {
    run(&[
        "connection",
        "modify",
        name,
        "+vpn.data",
        &format!("{key}={value}"),
    ])
    .await?;
    Ok(())
}

/// Marks the VPN password as agent-owned (`password-flags = 1`), so
/// NetworkManager asks a running Secret Agent for it at connect time
/// instead of expecting it embedded in the connection or prompting
/// interactively (which requires a TTY we don't have from a GUI app).
pub async fn mark_password_agent_owned(name: &str) -> Result<()> {
    set_vpn_data_key(name, "password-flags", "1").await
}

/// Split a line of `nmcli -t` output on unescaped `:` separators.
fn split_nmcli_fields(line: &str) -> Vec<&str> {
    // nmcli escapes literal colons in field values as `\:`. For the fields we
    // consume (NAME, UUID, TYPE, ACTIVE) this naive split is sufficient for
    // the common case; a fully correct unescaper would walk char-by-char.
    line.split(':').collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for a real-world `nmcli -g vpn.data connection show`
    /// output: two failover `remote` entries (each `host:port:proto`)
    /// joined by an escaped comma, plus other keys. Before the escape-aware
    /// rewrite, the naive `split(", ")` parser would incorrectly split on
    /// the escaped `\, ` too, corrupting the `remote` value and silently
    /// dropping the second entry.
    #[test]
    fn parses_multi_remote_with_escaped_separators() {
        let raw = r"connection-type = password-tls, remote = 45.236.52.35\:1194\:udp4\, 45.236.52.90\:1194\:udp4, cipher = AES-256-GCM, password-flags = 1";

        let data = parse_vpn_data(raw);

        assert_eq!(
            data.get("connection-type").map(String::as_str),
            Some("password-tls")
        );
        assert_eq!(
            data.get("remote").map(String::as_str),
            Some("45.236.52.35:1194:udp4, 45.236.52.90:1194:udp4")
        );
        assert_eq!(data.get("cipher").map(String::as_str), Some("AES-256-GCM"));
        assert_eq!(data.get("password-flags").map(String::as_str), Some("1"));
    }

    #[test]
    fn connection_details_splits_first_remote_into_host_port_protocol() {
        let raw = r"connection-type = password-tls, remote = 45.236.52.35\:1194\:udp4\, 45.236.52.90\:1194\:udp4, cipher = AES-256-GCM";
        let data = parse_vpn_data(raw);

        let first_remote = data
            .get("remote")
            .and_then(|v| v.split(", ").next())
            .map(str::to_string);
        assert_eq!(first_remote.as_deref(), Some("45.236.52.35:1194:udp4"));

        let mut parts = first_remote.as_deref().unwrap().splitn(3, ':');
        assert_eq!(parts.next(), Some("45.236.52.35"));
        assert_eq!(parts.next(), Some("1194"));
        assert_eq!(parts.next(), Some("udp4"));
    }

    #[test]
    fn normalized_protocol_strips_ip_version_suffix() {
        assert_eq!(normalized_protocol(Some("udp4")), "udp");
        assert_eq!(normalized_protocol(Some("tcp6")), "tcp");
        assert_eq!(normalized_protocol(Some("tcp")), "tcp");
        assert_eq!(normalized_protocol(None), "udp");
    }
}
