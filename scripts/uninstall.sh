#!/usr/bin/env bash
#
# uninstall.sh - Remove omarchy-openvpn-manager for the current user.
#
# Removes the installed binary, symlink, desktop entry, and icon. Does NOT
# remove any NetworkManager VPN connection profiles the user imported, since
# those are the user's data and may still be wanted even without the GUI.

set -euo pipefail

APP_NAME="omarchy-openvpn-manager"
INSTALL_DIR="${HOME}/.local/share/${APP_NAME}"
BIN_DIR="${HOME}/.local/bin"
DESKTOP_DIR="${HOME}/.local/share/applications"
ICON_DIR="${HOME}/.local/share/icons/hicolor/scalable/apps"
NETWORK_STATE_FILE="${INSTALL_DIR}/network-migration-state.json"

log() { printf '\033[1;34m[uninstall]\033[0m %s\n' "$1"; }
warn() { printf '\033[1;33m[uninstall]\033[0m %s\n' "$1" >&2; }

remove_path() {
  local path="$1"
  if [ -e "$path" ] || [ -L "$path" ]; then
    rm -rf "$path"
    log "Removed ${path}"
  fi
}

# Reads a boolean or string field out of the simple, fixed-format JSON written
# by install.sh's write_network_state(). Not a general JSON parser.
read_json_field() {
  local file="$1"
  local field="$2"
  grep -oP "\"${field}\"\s*:\s*\"?\K[^,\"\n}]+" "$file" 2>/dev/null | head -n1
}

maybe_disconnect_active_vpn() {
  command -v nmcli >/dev/null 2>&1 || return 0

  local active_name
  active_name="$(nmcli -t -f NAME,TYPE,ACTIVE connection show 2>/dev/null \
    | awk -F: '$2 == "vpn" && $3 == "yes" { print $1; exit }')"

  [ -n "$active_name" ] || return 0

  warn "The OpenVPN connection '${active_name}' is currently active."
  read -r -p "Disconnect it before uninstalling? [y/N] " reply
  case "$reply" in
    [yY] | [yY][eE][sS])
      log "Disconnecting '${active_name}'..."
      nmcli connection down "$active_name" >/dev/null 2>&1 || \
        warn "Failed to disconnect '${active_name}'; continuing uninstall anyway."
      ;;
    *)
      log "Leaving '${active_name}' connected."
      ;;
  esac
}

maybe_revert_networkd() {
  [ -f "${NETWORK_STATE_FILE}" ] || return 0

  local migrated
  migrated="$(read_json_field "${NETWORK_STATE_FILE}" migrated_by_installer)"
  if [ "$migrated" != "true" ]; then
    return 0
  fi

  local backup_dir
  backup_dir="$(read_json_field "${NETWORK_STATE_FILE}" backup_dir)"

  warn "This installer previously switched your system from systemd-networkd to NetworkManager."
  warn "Other Omarchy features (the top-bar network panel, 'omarchy network'/'omarchy-dns' commands,"
  warn "Wi-Fi QR sharing, band pinning) may now depend on NetworkManager if you've used them since."
  warn "Reverting to systemd-networkd could break those features."
  read -r -p "Revert this system back to systemd-networkd now? [y/N] " reply
  case "$reply" in
    [yY] | [yY][eE][sS]) ;;
    *)
      log "Leaving NetworkManager in place."
      return 0
      ;;
  esac

  log "Reverting to systemd-networkd..."
  sudo systemctl enable --now iwd.service >/dev/null 2>&1 || true
  sudo systemctl unmask systemd-networkd-wait-online.service >/dev/null 2>&1 || true
  sudo systemctl enable systemd-networkd-wait-online.service >/dev/null 2>&1 || true

  local unit
  for unit in systemd-networkd.service systemd-networkd.socket \
    systemd-networkd-varlink.socket systemd-networkd-varlink-metrics.socket \
    systemd-networkd-resolve-hook.socket; do
    sudo systemctl enable --now "$unit" >/dev/null 2>&1 || true
  done

  if [ -n "$backup_dir" ] && [ -d "$backup_dir" ]; then
    log "Restoring backed-up networkd config from ${backup_dir}..."
    sudo cp -a "${backup_dir}/." /etc/systemd/network/ 2>/dev/null || true
  fi

  sudo systemctl disable --now NetworkManager.service >/dev/null 2>&1 || true
  sudo systemctl restart systemd-networkd.service >/dev/null 2>&1 || true
  sudo systemctl restart systemd-resolved.service >/dev/null 2>&1 || true

  log "Reverted to systemd-networkd."
}

main() {
  maybe_disconnect_active_vpn
  maybe_revert_networkd

  remove_path "${BIN_DIR}/${APP_NAME}"
  remove_path "${INSTALL_DIR}"
  remove_path "${DESKTOP_DIR}/${APP_NAME}.desktop"
  remove_path "${ICON_DIR}/${APP_NAME}.svg"

  if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "${DESKTOP_DIR}" >/dev/null 2>&1 || true
  fi
  if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f "${HOME}/.local/share/icons/hicolor" >/dev/null 2>&1 || true
  fi

  log "Uninstall complete."
  log "Note: your imported OpenVPN connection profiles in NetworkManager were left untouched."
  log "To remove them too, use: nmcli connection show (to list) and nmcli connection delete <name>"
}

main "$@"
