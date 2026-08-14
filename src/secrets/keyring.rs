//! Secure storage for VPN passwords, backed by the freedesktop Secret
//! Service (gnome-keyring/libsecret via the `oo7` crate), instead of NM's
//! own connection-file secret storage. Passwords never touch our own
//! config files or NetworkManager's system connection files in plaintext.

use anyhow::{Context, Result};
use oo7::Keyring;

const SCHEMA_ATTRIBUTE: &str = "xdg:schema";
const SCHEMA_VALUE: &str = "org.omarchy.OpenvpnManager.Vpn";
const UUID_ATTRIBUTE: &str = "uuid";

fn attributes(uuid: &str) -> Vec<(&str, &str)> {
    vec![(SCHEMA_ATTRIBUTE, SCHEMA_VALUE), (UUID_ATTRIBUTE, uuid)]
}

/// Opens the default keyring and ensures its collection is unlocked before
/// use. Omarchy provisions a default keyring with `lock-on-idle=false` /
/// `lock-after=false` at first login, but the collection can still start
/// out in a locked state that must be explicitly unlocked over D-Bus before
/// any item can be created/read - otherwise operations fail with
/// `org.freedesktop.Secret.Error.IsLocked`.
async fn open_unlocked_keyring() -> Result<Keyring> {
    let keyring = Keyring::new().await.context("failed to open OS keyring")?;
    keyring
        .unlock()
        .await
        .context("failed to unlock OS keyring collection")?;
    Ok(keyring)
}

/// Store (or replace) the VPN password for a connection, keyed by its
/// NetworkManager UUID (not name, since names can be renamed/reused).
pub async fn store_password(uuid: &str, label: &str, password: &str) -> Result<()> {
    let keyring = open_unlocked_keyring().await?;
    let attrs = attributes(uuid);
    keyring
        .create_item(
            &format!("OpenVPN password for {label}"),
            &attrs,
            password.as_bytes(),
            true, // replace existing item for this uuid, if any
        )
        .await
        .context("failed to store password in OS keyring")?;
    Ok(())
}

/// Look up the stored VPN password for a connection by its UUID, if any.
pub async fn lookup_password(uuid: &str) -> Result<Option<String>> {
    let keyring = open_unlocked_keyring().await?;
    let attrs = attributes(uuid);
    let items = keyring
        .search_items(&attrs)
        .await
        .context("failed to search OS keyring")?;

    let Some(item) = items.into_iter().next() else {
        return Ok(None);
    };

    let secret = item
        .secret()
        .await
        .context("failed to read secret from OS keyring")?;
    let password = String::from_utf8(secret.as_bytes().to_vec())
        .context("stored VPN password was not valid UTF-8")?;
    Ok(Some(password))
}

/// Delete the stored VPN password for a connection, if any. Safe to call
/// even if nothing is stored.
pub async fn delete_password(uuid: &str) -> Result<()> {
    let keyring = open_unlocked_keyring().await?;
    let attrs = attributes(uuid);
    // `delete` returns an error if the backend can't be reached, but not if
    // simply no matching item exists, so this is safe to call unconditionally.
    let _ = keyring.delete(&attrs).await;
    Ok(())
}
