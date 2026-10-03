#!/usr/bin/env bash
# Optional local English ASR; source and weights stay outside the project repo.
set -euo pipefail
if [[ $# != 1 ]]; then
  echo 'Usage: scripts/setup-asr.sh PROVIDER_DIRECTORY' >&2
  exit 2
fi
provider_root=$(realpath -m "$1")
mkdir -p "$provider_root"
source="$provider_root/whisper"
revision=927cfce34f31707e17f2bff35c349632fb9e2c3a
if [[ ! -d "$source" ]]; then
  git clone https://github.com/ggml-org/whisper.cpp.git "$source"
  git -C "$source" checkout --detach "$revision"
fi
if [[ $(git -C "$source" rev-parse HEAD) != "$revision" ]] || [[ -n $(git -C "$source" status --porcelain --untracked-files=no) ]]; then
  echo 'Existing Whisper checkout differs from the qualified source; choose a new provider directory' >&2
  exit 1
fi
cmake -S "$source" -B "$source/build" -DWHISPER_BUILD_TESTS=OFF \
  -DWHISPER_BUILD_EXAMPLES=ON -DGGML_NATIVE=OFF -DGGML_BLAS=OFF \
  -DCMAKE_BUILD_TYPE=Release
cmake --build "$source/build" --target whisper-cli -j 4
model="$source/models/ggml-tiny.en.bin"
checksum=921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f
if [[ ! -f "$model" ]] || ! echo "$checksum  $model" | sha256sum -c - >/dev/null; then
  curl --fail --location --proto '=https' --tlsv1.2 \
    https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-tiny.en.bin \
    -o "$model"
fi
echo "$checksum  $model" | sha256sum -c -
"$source/build/bin/whisper-cli" --help
