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

log() { printf '\033[1;34m[uninstall]\033[0m %s\n' "$1"; }

remove_path() {
  local path="$1"
  if [ -e "$path" ] || [ -L "$path" ]; then
    rm -rf "$path"
    log "Removed ${path}"
  fi
}

main() {
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
