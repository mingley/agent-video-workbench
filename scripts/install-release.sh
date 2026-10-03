#!/usr/bin/env bash
set -euo pipefail
if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo 'Usage: scripts/install-release.sh NEW_BIN_DIRECTORY [VERSION]' >&2
  exit 2
fi
version=${2:-0.3.0}
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ || "$(uname -s)" != Linux ]]; then
  echo 'Choose a stable version and a supported Linux host' >&2
  exit 2
fi
arch=$(uname -m)
if [[ "$arch" != x86_64 && "$arch" != aarch64 ]]; then
  echo 'Supported CPU architectures: x86_64 and aarch64' >&2
  exit 2
fi
if [[ -e "$1/avw" ]]; then
  echo 'avw already exists; use a fresh versioned installation directory' >&2
  exit 2
fi
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
archive="avw-$version-linux-$arch.tar.gz"
base="https://github.com/mingley/agent-video-workbench/releases/download/v$version"
curl --fail --location --proto '=https' --proto-redir '=https' --tlsv1.2 "$base/$archive" -o "$work/$archive"
curl --fail --location --proto '=https' --proto-redir '=https' --tlsv1.2 "$base/SHA256SUMS" -o "$work/SHA256SUMS"
awk -v name="$archive" '$2 == name {print}' "$work/SHA256SUMS" > "$work/selected.sha256"
if [[ $(wc -l < "$work/selected.sha256") != 1 ]]; then
  echo 'Release checksum entry missing or ambiguous' >&2
  exit 2
fi
(cd "$work" && sha256sum -c selected.sha256)
# Our published bundles contain only regular files/directories beneath avw/.
while IFS= read -r name; do
  if [[ "$name" != avw/ && "$name" != avw/* ]] || [[ "$name" == *../* || "$name" == */.. ]]; then
    echo 'Release archive contains an unexpected path' >&2
    exit 2
  fi
done < <(tar -tzf "$work/$archive")
if tar -tvzf "$work/$archive" | awk 'substr($1,1,1)!="-" && substr($1,1,1)!="d" {bad=1} END{exit !bad}'; then
  echo 'Release archive contains an unexpected file type' >&2
  exit 2
fi
tar -xzf "$work/$archive" -C "$work"
"$work/avw/install.sh" "$1"
