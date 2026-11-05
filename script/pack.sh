#!/usr/bin/env bash
# Packs the six release binaries into the 0.5.1 release layout (spec/cli.md §4.1-4.2, spec/verbatim/cli/release.txt):
#   <out>/release/  12 archives + install.sh + install.ps1 + SHA256SUMS (the 15 GitHub release assets)
#   <out>/npm/      @anysola/code and the 12 @anysola/code-<target> tarballs
#
# Usage: script/pack.sh <version> <bin-dir> <out-dir>
#   <bin-dir>/<build>/anysola[.exe] for build in linux-x64 linux-arm64 darwin-x64 darwin-arm64 windows-x64
#   windows-arm64. Linux builds are static musl, so one binary fills the glibc and -musl names (ARCHITECTURE.md
#   "Release"). ANYSOLA_PACK_PARTIAL=1 skips missing builds (local testing only; the release needs all six).
# Needs GNU tar, gzip, zip, npm, sha256sum.
set -euo pipefail

[ "$#" -eq 3 ] || { echo "usage: $0 <version> <bin-dir> <out-dir>" >&2; exit 2; }
version=$1
bins=$(cd "$2" && pwd)
mkdir -p "$3"
out=$(cd "$3" && pwd)
root=$(cd "$(dirname "$0")/.." && pwd)
partial=${ANYSOLA_PACK_PARTIAL:-}
export TZ=UTC LC_ALL=C

# Target names in the order of the published 0.5.1 package.json; the build each one takes its bytes from.
targets="linux-x64 darwin-x64 linux-arm64 windows-x64 darwin-arm64 windows-arm64 linux-x64-musl linux-arm64-musl
linux-x64-baseline darwin-x64-baseline windows-x64-baseline linux-x64-baseline-musl"
build_of() {
  case "$1" in
    linux-x64*) echo linux-x64 ;;
    linux-arm64*) echo linux-arm64 ;;
    darwin-x64*) echo darwin-x64 ;;
    windows-x64*) echo windows-x64 ;;
    *) echo "$1" ;;
  esac
}
exe_of() { case "$1" in windows-*) echo anysola.exe ;; *) echo anysola ;; esac; }
os_of() { case "$1" in windows-*) echo win32 ;; darwin-*) echo darwin ;; *) echo linux ;; esac; }
cpu_of() { case "$1" in *-arm64*) echo arm64 ;; *) echo x64 ;; esac; }

rm -rf "$out/release" "$out/npm" "$out/pkg"
mkdir -p "$out/release" "$out/npm" "$out/pkg" "$out/archive-cache"
rm -f "$out/archive-cache"/*

# pack.ts wrote JSON.stringify output without a final newline.
no_final_newline() { printf '%s' "$(cat "$1")" > "$1.tmp" && mv "$1.tmp" "$1"; }

pack_dir() {
  chmod -R 755 "$1"
  (cd "$1" && npm pack --silent --pack-destination "$out/npm" >/dev/null)
}

for target in $targets; do
  build=$(build_of "$target")
  exe=$(exe_of "$target")
  src="$bins/$build/$exe"
  if [ ! -s "$src" ]; then
    [ -n "$partial" ] && { echo "skip $target (no $build build)" >&2; continue; }
    echo "missing binary: $src" >&2
    exit 1
  fi

  # Archive: made once per build, then copied, so the duplicate names carry identical bytes.
  case "$target" in linux-*) ext=tar.gz ;; *) ext=zip ;; esac
  cached="$out/archive-cache/$build.$ext"
  if [ ! -f "$cached" ]; then
    stage="$out/archive-cache/$build"
    mkdir -p "$stage"
    cp "$src" "$stage/$exe"
    chmod 755 "$stage/$exe"
    touch -d 1980-01-01T00:00:00Z "$stage/$exe"
    if [ "$ext" = tar.gz ]; then
      tar --mtime=@0 --owner=0 --group=0 --numeric-owner -czf "$cached" -C "$stage" "$exe"
    else
      (cd "$stage" && zip -X -q "$cached" "$exe")
    fi
  fi
  cp "$cached" "$out/release/anysola-$target.$ext"

  # Platform package @anysola/code-<target>.
  dir="$out/pkg/code-$target"
  mkdir -p "$dir/bin"
  cp "$src" "$dir/bin/$exe"
  cp "$root/LICENSE" "$root/NOTICE" "$dir/"
  cat > "$dir/package.json" <<EOF
{
  "name": "@anysola/code-$target",
  "version": "$version",
  "description": "Anysola Code binary, based on OpenCode (MIT)",
  "license": "MIT",
  "repository": {
    "type": "git",
    "url": "git+https://github.com/Anysola/code.git"
  },
  "os": [
    "$(os_of "$target")"
  ],
  "cpu": [
    "$(cpu_of "$target")"
  ]
}
EOF
  no_final_newline "$dir/package.json"
  pack_dir "$dir"
done

# Main package @anysola/code: a Node launcher runs the platform binary; the optional postinstall
# puts the binary in the launcher's place on Linux and macOS.
dir="$out/pkg/code"
mkdir -p "$dir/bin"
cp "$root/script/npm/postinstall.mjs" "$root/script/npm/resolve.cjs" "$root/LICENSE" "$root/NOTICE" "$dir/"
cp "$root/script/npm/anysola" "$dir/bin/anysola"
chmod 755 "$dir/bin/anysola"
deps=""
for target in $targets; do
  deps="$deps${deps:+,
}    \"@anysola/code-$target\": \"$version\""
done
cat > "$dir/package.json" <<EOF
{
  "name": "@anysola/code",
  "bin": {
    "anysola": "./bin/anysola"
  },
  "scripts": {
    "postinstall": "node ./postinstall.mjs"
  },
  "version": "$version",
  "description": "Anysola Code: the Anysola coding agent for your terminal, based on OpenCode (MIT)",
  "license": "MIT",
  "repository": {
    "type": "git",
    "url": "git+https://github.com/Anysola/code.git"
  },
  "homepage": "https://www.anysola.com/code",
  "os": [
    "darwin",
    "linux",
    "win32"
  ],
  "cpu": [
    "arm64",
    "x64"
  ],
  "optionalDependencies": {
$deps
  }
}
EOF
no_final_newline "$dir/package.json"
pack_dir "$dir"
rm -rf "$out/archive-cache"

# Release assets: install scripts unchanged, SHA256SUMS as `sha256sum anysola-* install.sh install.ps1`.
cp "$root/install" "$out/release/install.sh"
cp "$root/install.ps1" "$out/release/install.ps1"
(cd "$out/release" && sha256sum anysola-* install.sh install.ps1 > SHA256SUMS)

# Self-check: asset names, single 0755 entry per archive, duplicates byte-identical.
fail=0
for target in $targets; do
  case "$target" in linux-*) f="anysola-$target.tar.gz" ;; *) f="anysola-$target.zip" ;; esac
  [ -f "$out/release/$f" ] || { [ -n "$partial" ] || { echo "FAIL: missing $f" >&2; fail=1; }; continue; }
  case "$f" in
    *.tar.gz) listing=$(tar tzvf "$out/release/$f"); entry=$(tar tzf "$out/release/$f") ;;
    *) listing=$(zipinfo "$out/release/$f" | sed -n '3,$p' | grep -v '^[0-9]* file'); entry=$(zipinfo -1 "$out/release/$f") ;;
  esac
  [ "$entry" = "$(exe_of "$target")" ] || { echo "FAIL: $f entries: $entry" >&2; fail=1; }
  case "$listing" in -rwxr-xr-x*) ;; *) echo "FAIL: $f mode: $listing" >&2; fail=1 ;; esac
  base="anysola-$(build_of "$target").${f#*.}"
  [ "$f" = "$base" ] || cmp -s "$out/release/$f" "$out/release/$base" || { echo "FAIL: $f differs from $base" >&2; fail=1; }
done
count=$(find "$out/release" -type f | wc -l)
[ -n "$partial" ] || [ "$count" -eq 15 ] || { echo "FAIL: $count release assets, expected 15" >&2; fail=1; }
ls -l "$out/release" "$out/npm"
cat "$out/release/SHA256SUMS"
exit "$fail"
