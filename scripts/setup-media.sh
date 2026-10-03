#!/usr/bin/env bash
# Verified FFmpeg 8 CPU backend for the qualified Linux x86_64 route.
set -euo pipefail
if [[ $# != 1 || $(uname -s) != Linux || $(uname -m) != x86_64 ]]; then
  echo 'Usage: scripts/setup-media.sh TOOL_DIRECTORY (Linux x86_64)' >&2
  exit 2
fi
media_root=$(realpath -m "$1")
mkdir -p "$media_root"
archive="$media_root/ffmpeg8.zip"
checksum=ca75b05e887c7a97676632f673031875847be83daa9794298fed9cef8cac14ad
if [[ ! -f "$archive" ]] || ! echo "$checksum  $archive" | sha256sum -c - >/dev/null; then
  curl --fail --location --proto '=https' --tlsv1.2 \
    https://media.githubusercontent.com/media/zackees/ffmpeg_bins/df95abcb0ce6efff710dda5ef28a2f6f1dc21493/v8.0/linux.zip \
    -o "$archive"
fi
echo "$checksum  $archive" | sha256sum -c -
python3 - "$media_root" <<'PY'
from pathlib import Path
import hashlib, os, shutil, sys, tempfile, zipfile
root = Path(sys.argv[1])
with zipfile.ZipFile(root / 'ffmpeg8.zip') as archive:
    if set(archive.namelist()) != {'linux/', 'linux/ffmpeg', 'linux/ffprobe'}:
        raise ValueError('unexpected FFmpeg archive entries')
    destination = root / 'ffmpeg8/linux'
    destination.mkdir(parents=True, exist_ok=True)
    for name in ('ffmpeg', 'ffprobe'):
        path = destination / name
        with archive.open('linux/' + name) as source, tempfile.NamedTemporaryFile(dir=destination, delete=False) as staged:
            temporary = Path(staged.name)
            shutil.copyfileobj(source, staged)
            staged.flush()
            os.fsync(staged.fileno())
        try:
            with temporary.open('rb') as source:
                expected = hashlib.file_digest(source, 'sha256').digest()
            actual = None
            if path.is_file():
                with path.open('rb') as source:
                    actual = hashlib.file_digest(source, 'sha256').digest()
            temporary.chmod(0o755)
            if actual != expected:
                os.replace(temporary, path)
            else:
                path.chmod(0o755)
        finally:
            temporary.unlink(missing_ok=True)

PY
"$media_root/ffmpeg8/linux/ffmpeg" -version | head -n 1
"$media_root/ffmpeg8/linux/ffprobe" -version | head -n 1
