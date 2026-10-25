#!/usr/bin/env bash
# Fetch a pinned, checksummed llama.cpp build for this platform.
#   scripts/fetch-llama.sh                 # dev: installs into ~/.burrow/llama (burrowd finds it there)
#   scripts/fetch-llama.sh --bundle        # app: apps/desktop/src-tauri/resources/llama
#   scripts/fetch-llama.sh --vulkan        # Linux/Windows GPU build instead of CPU
# Runs on macOS, Linux and Windows (Git Bash, as on GitHub's Windows runners).
set -euo pipefail

TAG=b11552
dest="${BURROW_HOME:-$HOME/.burrow}/llama"
flavor=cpu
for a in "$@"; do
  case "$a" in
    --bundle) dest="$(cd "$(dirname "$0")/.." && pwd)/apps/desktop/src-tauri/resources/llama" ;;
    --vulkan) flavor=vulkan ;;
    *) echo "unknown option $a" >&2; exit 2 ;;
  esac
done

os="$(uname -s)"; arch="$(uname -m)"
case "$os/$arch/$flavor" in
  Darwin/arm64/*)            asset=macos-arm64.tar.gz;       sum=f4b33fe40d098033fb1a19a8a4a84383c4ca84c37b6a60d9f9a253f349b48c07 ;;
  Darwin/x86_64/*)           asset=macos-x64.tar.gz;         sum=b7977620cf5c30b8905caa45d7dbcc08766b5d4a94b3cde66ecf0323a495096f ;;
  Linux/x86_64/cpu)          asset=ubuntu-x64.tar.gz;        sum=a45a6afda05d08028bd37998072ab602c96799bccde126835941623cb99bb727 ;;
  Linux/x86_64/vulkan)       asset=ubuntu-vulkan-x64.tar.gz; sum=c7bb933f3584b73296fbadea6e0d6273ddb7907699ec59ef08d0bb59e0a8b997 ;;
  Linux/aarch64/cpu)         asset=ubuntu-arm64.tar.gz;      sum=8c5e5074c4e303308367471941b70a0f1fac4a1a35c7c67c0bec895d8df092f9 ;;
  MINGW*/x86_64/cpu|MSYS*/x86_64/cpu)       asset=win-cpu-x64.zip;    sum=c00741179811ff903b515eae28bdeef1f47af6e81c49920c4e68b62954e056d6 ;;
  MINGW*/x86_64/vulkan|MSYS*/x86_64/vulkan) asset=win-vulkan-x64.zip; sum=83dc16325b532ba603034719e8377e28ca5093c5d4370141a6522f1a0250386c ;;
  MINGW*/aarch64/cpu|MSYS*/aarch64/cpu)     asset=win-cpu-arm64.zip;  sum=f4f181cca934b676c276d488d89a22e4d140f678332c43611f7f85c7fa0aedae ;;
  *) echo "no pinned llama.cpp build for $os/$arch/$flavor" >&2; exit 1 ;;
esac

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
file="llama-$TAG-bin-$asset"
curl -fsSL -o "$tmp/$file" "https://github.com/ggml-org/llama.cpp/releases/download/$TAG/$file"
got="$( (sha256sum "$tmp/$file" 2>/dev/null || shasum -a 256 "$tmp/$file") | cut -d' ' -f1)"
[ "$got" = "$sum" ] || { echo "checksum mismatch for $file: $got" >&2; exit 1; }

mkdir -p "$tmp/x"
case "$asset" in
  *.zip) unzip -q "$tmp/$file" -d "$tmp/x" ;;
  *)     tar xzf "$tmp/$file" -C "$tmp/x" ;;
esac
# archives either hold a top-level llama-<tag>/ folder or the files directly
src="$tmp/x"; [ -d "$tmp/x/llama-$TAG" ] && src="$tmp/x/llama-$TAG"
rm -rf "$dest"; mkdir -p "$(dirname "$dest")"; mv "$src" "$dest"
echo "llama.cpp $TAG ($asset) installed in $dest"
