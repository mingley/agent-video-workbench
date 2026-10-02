#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ $# != 1 ]]; then
  echo 'Usage: scripts/package.sh NEW_OUTPUT_DIRECTORY' >&2
  exit 2
fi
output=$(realpath -m "$1")
if [[ -e "$output" ]]; then
  echo 'Output directory already exists' >&2
  exit 2
fi
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo build --release --locked
mkdir -p "$output/avw/bin"
cp target/release/avw "$output/avw/bin/avw"
cp LICENSE AGENTCUT-LICENSE DEVELOPMENT.md "$output/avw/"
# Record dependency license declarations; FFmpeg/font/model assets are external.
cargo metadata --locked --format-version 1 | python3 -c '
import json,sys
metadata=json.load(sys.stdin)
packages=[{"name":p["name"],"version":p["version"],"license":p["license"],"repository":p["repository"]} for p in metadata["packages"]]
json.dump(sorted(packages,key=lambda p:(p["name"],p["version"])),sys.stdout,indent=2)
' > "$output/avw/dependency-licenses.json"
cp scripts/install-binary.sh "$output/avw/install.sh"
archive="avw-$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m).tar.gz"
tar -C "$output" -czf "$output/$archive" avw
(cd "$output" && sha256sum "$archive" > SHA256SUMS)
echo "$output/$archive"
