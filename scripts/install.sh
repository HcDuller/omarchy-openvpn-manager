#!/usr/bin/env bash
#
# install.sh - Install omarchy-openvpn-manager for the current user.
#
# Installs runtime dependencies via pacman (Arch/Omarchy only), fetches a
# prebuilt release binary matching the host architecture when available
# (falling back to building from source with cargo), and installs the
# binary, desktop entry, and icon into the user's home directory. No root
# privileges are required beyond the pacman dependency install step.

set -euo pipefail

APP_NAME="omarchy-openvpn-manager"
REPO="HcDuller/omarchy-openvpn-manager"
INSTALL_DIR="${HOME}/.local/share/${APP_NAME}"
BIN_DIR="${HOME}/.local/bin"
DESKTOP_DIR="${HOME}/.local/share/applications"
ICON_DIR="${HOME}/.local/share/icons/hicolor/scalable/apps"
NETWORK_STATE_FILE="${INSTALL_DIR}/network-migration-state.json"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

log() { printf '\033[1;34m[install]\033[0m %s\n' "$1"; }
warn() { printf '\033[1;33m[install]\033[0m %s\n' "$1" >&2; }
err() { printf '\033[1;31m[install]\033[0m %s\n' "$1" >&2; }

require_arch() {
  if ! command -v pacman >/dev/null 2>&1; then
    err "pacman not found. This installer targets Arch/Omarchy only."
    exit 1
  fi
}

install_dependencies() {
  log "Installing runtime dependencies via pacman..."
  local deps=(networkmanager networkmanager-openvpn gtk4 libadwaita)
  local missing=()
  for dep in "${deps[@]}"; do
    if ! pacman -Qi "$dep" >/dev/null 2>&1; then
      missing+=("$dep")
    fi
  done

  if [ "${#missing[@]}" -gt 0 ]; then
    log "Missing packages: ${missing[*]}"
    sudo pacman -S --needed --noconfirm "${missing[@]}"
  else
    log "All runtime dependencies already installed."
  fi
}

detect_arch_triple() {
  case "$(uname -m)" in
    x86_64) echo "x86_64-unknown-linux-gnu" ;;
    aarch64) echo "aarch64-unknown-linux-gnu" ;;
    *) echo "" ;;
  esac
}

# --- Network stack migration (systemd-networkd -> NetworkManager) ---
#
# Older Omarchy installs (and archinstall's "copy ISO network" mode) default
# to systemd-networkd + iwd. This tool requires NetworkManager (via nmcli),
# so if networkd is what's actually running, offer to migrate using the same
# approach Omarchy's own upgrade migration uses, and record whether we did so
# in a state file so uninstall.sh can offer to revert it later.

networkd_units=(
  systemd-networkd.service
  systemd-networkd.socket
  systemd-networkd-varlink.socket
  systemd-networkd-varlink-metrics.socket
  systemd-networkd-resolve-hook.socket
)

is_networkd_active() {
  systemctl is-active --quiet systemd-networkd.service 2>/dev/null
}

is_networkmanager_active() {
  systemctl is-active --quiet NetworkManager.service 2>/dev/null
}

write_network_state() {
  local migrated="$1"
  local networkd_was_active="$2"
  local backup_dir="${3:-}"
  mkdir -p "${INSTALL_DIR}"
  cat > "${NETWORK_STATE_FILE}" <<EOF
{
  "migrated_by_installer": ${migrated},
  "networkd_was_active": ${networkd_was_active},
  "backup_dir": "${backup_dir}"
}
EOF
}

stock_networkd_file() {
  local file="$1"
  [ -f "$file" ] || return 1
  case "$(basename "$file")" in
    20-ethernet.network | 20-wlan.network | 20-wwan.network) ;;
    *) return 1 ;;
  esac
  grep -Eq '^[[:space:]]*DHCP=yes[[:space:]]*$' "$file" || return 1
  grep -Eq '^[[:space:]]*Name=(en\*|eth\*|wl\*|ww\*)[[:space:]]*$' "$file" || return 1
}

backup_stock_networkd_files() {
  local backup_dir="$1"
  local file
  for file in /etc/systemd/network/20-ethernet.network \
    /etc/systemd/network/20-wlan.network \
    /etc/systemd/network/20-wwan.network; do
    if stock_networkd_file "$file"; then
      sudo install -d -m 0755 "$backup_dir"
      sudo mv "$file" "$backup_dir/"
    fi
  done
}

maybe_migrate_networkd() {
  if is_networkmanager_active; then
    log "NetworkManager is already active; no network stack migration needed."
    write_network_state "false" "false" ""
    return
  fi

  if ! is_networkd_active; then
    # Neither is active/detected in a way we can confirm; NetworkManager was
    # just installed above and will be enabled normally by systemd on next
    # boot/login. Nothing to migrate away from.
    write_network_state "false" "false" ""
    return
  fi

  warn "This system is currently using systemd-networkd, but ${APP_NAME} requires NetworkManager."
  warn "Switching to NetworkManager also enables other Omarchy features that assume it,"
  warn "such as the top-bar network panel, 'omarchy network'/'omarchy-dns' commands, and Wi-Fi QR sharing."
  read -r -p "Switch this system from systemd-networkd to NetworkManager now? [y/N] " reply
  case "$reply" in
    [yY] | [yY][eE][sS]) ;;
    *)
      err "Cannot continue without NetworkManager. Aborting install."
      exit 1
      ;;
  esac

  log "Enabling NetworkManager..."
  sudo systemctl enable --now NetworkManager.service

  if ! is_networkmanager_active; then
    err "NetworkManager did not come up successfully; aborting before touching networkd."
    exit 1
  fi

  log "NetworkManager is carrying the link; retiring systemd-networkd..."
  local backup_dir
  backup_dir="/etc/systemd/network/omarchy-networkd-retired-$(date +%Y%m%d%H%M%S)"

  local unit
  for unit in "${networkd_units[@]}"; do
    sudo systemctl disable --now "$unit" >/dev/null 2>&1 || true
  done
  sudo systemctl disable systemd-networkd-wait-online.service >/dev/null 2>&1 || true
  sudo systemctl mask systemd-networkd-wait-online.service >/dev/null 2>&1 || true
  sudo systemctl disable --now iwd.service >/dev/null 2>&1 || true

  backup_stock_networkd_files "$backup_dir"

  sudo systemctl stop systemd-networkd.service >/dev/null 2>&1 || true
  sudo systemctl reload NetworkManager.service >/dev/null 2>&1 || true
  sudo systemctl restart systemd-resolved.service >/dev/null 2>&1 || true

  log "Migrated from systemd-networkd to NetworkManager (backup: ${backup_dir})."
  write_network_state "true" "true" "$backup_dir"
}

fetch_prebuilt_binary() {
  local triple
  triple="$(detect_arch_triple)"
  if [ -z "$triple" ]; then
    warn "No prebuilt binary available for architecture $(uname -m)."
    return 1
  fi

  if ! command -v curl >/dev/null 2>&1; then
    warn "curl not found, cannot fetch prebuilt release."
    return 1
  fi

  local asset="${APP_NAME}-${triple}.tar.gz"
  local url="https://github.com/${REPO}/releases/latest/download/${asset}"

  log "Attempting to download prebuilt binary: ${url}"
  local tmp
  tmp="$(mktemp -d)"
  if curl -fsSL "$url" -o "${tmp}/${asset}" 2>/dev/null; then
    tar -xzf "${tmp}/${asset}" -C "${tmp}"
    if [ -f "${tmp}/${APP_NAME}" ]; then
      mkdir -p "${INSTALL_DIR}"
      install -m 755 "${tmp}/${APP_NAME}" "${INSTALL_DIR}/${APP_NAME}"
      rm -rf "${tmp}"
      log "Installed prebuilt binary."
      return 0
    fi
  fi

  rm -rf "${tmp}"
  warn "No prebuilt release found, falling back to building from source."
  return 1
}

build_from_source() {
  if ! command -v cargo >/dev/null 2>&1; then
    log "Rust toolchain not found, installing via pacman..."
    sudo pacman -S --needed --noconfirm rust
  fi

  log "Building ${APP_NAME} from source (this may take a minute)..."
  (cd "${REPO_ROOT}" && cargo build --release)

  mkdir -p "${INSTALL_DIR}"
  install -m 755 "${REPO_ROOT}/target/release/${APP_NAME}" "${INSTALL_DIR}/${APP_NAME}"
}

install_binary() {
  if fetch_prebuilt_binary; then
    return
  fi
  build_from_source
}

install_desktop_entry() {
  log "Installing desktop entry and icon..."
  mkdir -p "${DESKTOP_DIR}" "${ICON_DIR}"
  install -m 644 "${REPO_ROOT}/assets/${APP_NAME}.desktop" "${DESKTOP_DIR}/${APP_NAME}.desktop"
  install -m 644 "${REPO_ROOT}/assets/icons/${APP_NAME}.svg" "${ICON_DIR}/${APP_NAME}.svg"

  if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "${DESKTOP_DIR}" >/dev/null 2>&1 || true
  fi
  if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f "${HOME}/.local/share/icons/hicolor" >/dev/null 2>&1 || true
  fi
}

link_binary() {
  mkdir -p "${BIN_DIR}"
  ln -sf "${INSTALL_DIR}/${APP_NAME}" "${BIN_DIR}/${APP_NAME}"
  log "Symlinked binary to ${BIN_DIR}/${APP_NAME}"

  case ":${PATH}:" in
    *":${BIN_DIR}:"*) ;;
    *) warn "${BIN_DIR} is not in your PATH. Add it to your shell profile to run '${APP_NAME}' directly." ;;
  esac
}

main() {
  require_arch
  install_dependencies
  maybe_migrate_networkd
  install_binary
  link_binary
  install_desktop_entry
  log "Installation complete. Launch '${APP_NAME}' from your app launcher or run: ${APP_NAME}"
}

main "$@"
