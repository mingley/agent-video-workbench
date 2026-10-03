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
python3 scripts/check-schema.py target/release/avw
mkdir -p "$output/avw/bin"
cp target/release/avw "$output/avw/bin/avw"
cp LICENSE DEVELOPMENT.md AGENT_GUIDE.md "$output/avw/"
cp README.md "$output/avw/"
cp -R docs examples specs licenses "$output/avw/"
mkdir -p "$output/avw/evaluation"
cp evaluation/README.md evaluation/candidates.lock.json "$output/avw/evaluation/"
cp -R evaluation/results evaluation/service-results "$output/avw/evaluation/"
python3 - "$output" <<'PY'
import json, platform, subprocess, sys
from pathlib import Path
root = Path(sys.argv[1]) / 'avw'
metadata = {'version':subprocess.check_output([str(root/'bin/avw'),'--version'],text=True).strip(),
            'commit':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),
            'rust':subprocess.check_output(['rustc','--version'],text=True).strip(),
            'platform':platform.platform(), 'externalRuntime':['FFmpeg/ffprobe 8','licensed TTF font'],
            'optionalRuntime':['whisper.cpp v1.9.4 and checksum-verified English model']}
(root / 'release.json').write_text(json.dumps(metadata,indent=2)+'\n')
PY
# Record dependency license declarations; FFmpeg/font/model assets are external.
cargo metadata --locked --format-version 1 | python3 -c '
import json,sys
metadata=json.load(sys.stdin)
packages=[{"name":p["name"],"version":p["version"],"license":p["license"],"repository":p["repository"]} for p in metadata["packages"]]
json.dump(sorted(packages,key=lambda p:(p["name"],p["version"])),sys.stdout,indent=2)
' > "$output/avw/dependency-licenses.json"
mkdir -p "$output/avw/scripts"
cp scripts/setup-media.sh scripts/setup-asr.sh scripts/install-release.sh "$output/avw/scripts/"
cp scripts/install-binary.sh "$output/avw/install.sh"
(cd "$output/avw" && sha256sum bin/avw > SHA256SUMS)
version=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["packages"][0]["version"])')
archive="avw-$version-$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m).tar.gz"
tar -C "$output" -czf "$output/$archive" avw
(cd "$output" && sha256sum "$archive" > SHA256SUMS)
echo "$output/$archive"
