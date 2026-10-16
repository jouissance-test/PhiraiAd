#!/usr/bin/env bash
# No installation, downloads, deletion, or changes to the system linker setup.
set -euo pipefail
audit_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
audit_support="$audit_root/tools/preview-audit/host-support"
mkdir -p "$audit_support/lib"
for spec in 'asound:libasound.so.2' 'wayland-client:libwayland-client.so.0' 'wayland-cursor:libwayland-cursor.so.0'; do
  audit_name="${spec%%:*}"
  audit_soname="${spec#*:}"
  audit_source="$(ldconfig -p | awk -v soname="$audit_soname" '$1 == soname && /x86-64/ && !seen++ { print $NF }')"
  if [[ ! -f "$audit_source" ]]; then printf 'Required host runtime missing: %s\n' "$audit_soname" >&2; exit 1; fi
  if [[ ! -e "$audit_support/lib/lib$audit_name.so" ]]; then ln -s "$audit_source" "$audit_support/lib/lib$audit_name.so"; fi
done
export PKG_CONFIG_PATH="$audit_support/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
cd "$audit_root"
cargo build --offline -p phira-preview-audit "$@"
