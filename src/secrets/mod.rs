//! Secure credential storage and NetworkManager Secret Agent integration.
//!
//! VPN passwords are stored in the user's OS keyring (via `oo7`, the
//! freedesktop Secret Service) rather than in NetworkManager's own
//! connection files or anywhere in this app's own config. We register as a
//! NetworkManager Secret Agent so NM can ask us for the password at connect
//! time, which is what lets `nmcli connection up`/GUI-triggered connects
//! succeed without `--ask` (which requires a TTY we don't have) or an
//! embedded plaintext password.

pub mod agent;
pub mod keyring;
