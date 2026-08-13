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
  install_binary
  link_binary
  install_desktop_entry
  log "Installation complete. Launch '${APP_NAME}' from your app launcher or run: ${APP_NAME}"
}

main "$@"
