# omarchy-openvpn-manager

A GTK4 / Libadwaita OpenVPN connection manager for [Omarchy](https://omarchy.org),
built on top of NetworkManager (`nmcli`) with a tray icon and optional waybar
integration.

Omarchy's current default network stack is NetworkManager, but older
installs (or systems provisioned via archinstall's "copy ISO network" mode)
may still be running `systemd-networkd` + `iwd`. Since this tool requires
NetworkManager, `install.sh` detects that case and offers to migrate for you
(see [Network stack migration](#network-stack-migration-systemd-networkd--networkmanager)
below).

## Features

- Import `.ovpn` files as NetworkManager connection profiles
- List, connect, disconnect, and delete OpenVPN profiles
- Live connection status via a NetworkManager D-Bus watcher (no polling)
- System tray icon (StatusNotifierItem) with quick connect/disconnect
- Optional opt-in waybar `custom/` module script
- Credentials are handled by NetworkManager's own secret storage
  (libsecret/gnome-keyring), which Omarchy already provisions per user —
  no custom credential storage is implemented by this app

## Requirements

- Arch Linux / Omarchy
- `NetworkManager` + `networkmanager-openvpn` (installed automatically if missing)
- GTK4 + Libadwaita (installed automatically if missing)

## Install

```sh
git clone https://github.com/HcDuller/omarchy-openvpn-manager.git
cd omarchy-openvpn-manager
./scripts/install.sh
```

The installer will:

1. Install missing pacman dependencies (`networkmanager`,
   `networkmanager-openvpn`, `gtk4`, `libadwaita`)
2. Try to download a prebuilt binary from the latest GitHub Release matching
   your architecture; if none is available it falls back to
   `cargo build --release` (installing the Rust toolchain if needed)
3. Install the binary to `~/.local/share/omarchy-openvpn-manager/` and
   symlink it into `~/.local/bin/`
4. Install a desktop entry and icon so the app shows up in your launcher

Make sure `~/.local/bin` is on your `PATH`.

## Network stack migration (systemd-networkd → NetworkManager)

This tool requires NetworkManager to be the active network stack, since it
drives everything through `nmcli`. If `install.sh` detects that your system
is still running `systemd-networkd` (common on older Omarchy installs or
archinstall's "copy ISO network" mode) instead of NetworkManager, it will:

1. Explain that switching also enables other Omarchy features that assume
   NetworkManager (the top-bar network panel, `omarchy network`/`omarchy-dns`
   commands, Wi-Fi QR sharing, Wi-Fi band pinning) — not just this app
2. Ask for explicit confirmation before changing anything
3. If confirmed, perform the same migration Omarchy's own internal upgrade
   path uses: enable NetworkManager, confirm it's actually carrying the
   network link, then disable/mask `systemd-networkd`/`iwd`, back up any
   stock `.network` files it finds under `/etc/systemd/network/`, and
   restart `systemd-resolved`
4. Record whether it performed this migration in
   `~/.local/share/omarchy-openvpn-manager/network-migration-state.json`

If NetworkManager is already active (the common case on current Omarchy
installs), none of this runs — install proceeds normally.

**Uninstalling**: `uninstall.sh` checks that state file. If `install.sh`
performed the migration, it will ask (again, with an explicit warning about
the other Omarchy features that may now depend on NetworkManager) whether you
want to revert back to `systemd-networkd`. If you decline, or if
NetworkManager was already active before you installed this app,
`uninstall.sh` leaves your network stack untouched.

## Uninstall

```sh
./scripts/uninstall.sh
```

This removes the binary, symlink, desktop entry, and icon. It does **not**
delete any OpenVPN connection profiles you've imported into NetworkManager —
those are your data. To remove them too:

```sh
nmcli connection show
nmcli connection delete <profile-name>
```

## Waybar integration (optional)

A status/toggle script is provided at `scripts/waybar-module.sh` but is not
installed automatically. To use it, add something like this to your waybar
config:

```jsonc
"custom/openvpn": {
  "exec": "~/.local/share/omarchy-openvpn-manager/waybar-module.sh",
  "return-type": "json",
  "interval": 5,
  "on-click": "~/.local/share/omarchy-openvpn-manager/waybar-module.sh --toggle"
}
```

Then add `"custom/openvpn"` to one of your bar's module lists. Note the tray
icon (via `ksni`/StatusNotifierItem) works out of the box with waybar's
built-in `tray` module without any config changes, if you'd rather use that.

## Development

```sh
cargo build
cargo run
```

Project layout:

```
src/
  main.rs   # wires the GUI, tray, and D-Bus watcher together
  nm/       # NetworkManager integration (nmcli wrapper + D-Bus watcher)
  ui/       # relm4 GTK4/Libadwaita GUI
  tray/     # ksni-based StatusNotifierItem tray icon
scripts/
  install.sh          # user-local installer
  uninstall.sh        # user-local uninstaller
  waybar-module.sh    # optional waybar custom module (opt-in)
assets/
  omarchy-openvpn-manager.desktop
  icons/
```

## License

MIT
