#!/usr/bin/env bash
# Fetch a pinned, checksummed llama.cpp build for this platform.
#   scripts/fetch-llama.sh                 # dev: installs into ~/.burrow/llama (burrowd finds it there)
#   scripts/fetch-llama.sh --bundle        # app: apps/desktop/src-tauri/resources/llama
#   scripts/fetch-llama.sh --vulkan        # Linux/Windows GPU build instead of CPU
# Runs on macOS, Linux and Windows (Git Bash, as on GitHub's Windows runners).
set -euo pipefail

pins="$(dirname "$0")/../crates/burrowd/llama-pins.txt" # shared with the daemon, which downloads it on demand
TAG="$(head -1 "$pins")"
dest="${BURROW_HOME:-$HOME/.burrow}/llama"
flavor=cpu
for a in "$@"; do
  case "$a" in
    --bundle) dest="$(cd "$(dirname "$0")/.." && pwd)/apps/desktop/src-tauri/resources/llama" ;;
    --vulkan) flavor=vulkan ;;
    *) echo "unknown option $a" >&2; exit 2 ;;
  esac
done

case "$(uname -s)" in Darwin) os=macos ;; Linux) os=linux ;; *) os=windows ;; esac
arch="$(uname -m)"; [ "$arch" = arm64 ] && arch=aarch64
read -r asset sum < <(awk -v k="$os/$arch/$flavor" '$1 == k { print $2, $3 }' "$pins")
[ -n "${asset:-}" ] || { echo "no pinned llama.cpp build for $os/$arch/$flavor" >&2; exit 1; }

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
