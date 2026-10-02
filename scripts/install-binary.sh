#!/usr/bin/env bash
set -euo pipefail
if [[ $# != 1 ]]; then
  echo 'Usage: install.sh INSTALL_DIRECTORY' >&2
  exit 2
fi
bundle=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$1"
# Refuse to replace an existing installation without an explicit user action.
if [[ -e "$1/avw" ]]; then
  echo 'avw already exists in the installation directory' >&2
  exit 2
fi
cp "$bundle/bin/avw" "$1/avw"
chmod 755 "$1/avw"
"$1/avw" --version
