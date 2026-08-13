#!/usr/bin/env bash
#
# waybar-module.sh - Optional status script for a waybar `custom/` module.
#
# This is opt-in and not installed automatically by install.sh. To use it,
# add a module to your waybar config (typically ~/.config/waybar/config.jsonc)
# pointing at this script, for example:
#
#   "custom/openvpn": {
#     "exec": "~/.local/share/omarchy-openvpn-manager/waybar-module.sh",
#     "return-type": "json",
#     "interval": 5,
#     "on-click": "~/.local/share/omarchy-openvpn-manager/waybar-module.sh --toggle"
#   }
#
# And reference "custom/openvpn" in one of your waybar bars' module lists.
#
# Output is a single JSON line consumed by waybar's `custom` module type:
# https://github.com/Alexays/Waybar/wiki/Module:-Custom

set -euo pipefail

active_profile() {
  nmcli -t -f NAME,TYPE,ACTIVE connection show 2>/dev/null \
    | awk -F: '$2 == "vpn" && $3 == "yes" { print $1; exit }'
}

toggle() {
  local name
  name="$(active_profile)"
  if [ -n "$name" ]; then
    nmcli connection down "$name" >/dev/null 2>&1 || true
  else
    # No active profile to toggle off; connect the first known openvpn
    # profile as a reasonable default for a single-profile setup.
    name="$(nmcli -t -f NAME,TYPE connection show 2>/dev/null | awk -F: '$2 == "vpn" { print $1; exit }')"
    if [ -n "$name" ]; then
      nmcli connection up "$name" >/dev/null 2>&1 || true
    fi
  fi
}

status_json() {
  local name
  name="$(active_profile)"
  if [ -n "$name" ]; then
    printf '{"text":"VPN: %s","tooltip":"Connected to %s","class":"connected"}\n' "$name" "$name"
  else
    printf '{"text":"VPN: off","tooltip":"No active OpenVPN connection","class":"disconnected"}\n'
  fi
}

case "${1:-}" in
  --toggle)
    toggle
    ;;
  *)
    status_json
    ;;
esac
