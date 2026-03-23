#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

fix_nbis_lib64_symlinks() {
  local profile="$1"
  local fixed=0
  shopt -s nullglob
  for d in "target/$profile"/build/nbis-rs-*/out/build/install_staging/nfiq2; do
    if [[ -d "$d/lib64" && ! -e "$d/lib" ]]; then
      ln -s lib64 "$d/lib"
      echo "  linked $d/lib -> lib64"
      fixed=1
    fi
  done
  shopt -u nullglob
  return "$fixed"
}

echo "[1/4] Building release library..."
if ! cargo build --release; then
  echo "release build failed, attempting nbis lib64 -> lib symlink fix..."
  fix_nbis_lib64_symlinks release || true
  cargo build --release
fi
fix_nbis_lib64_symlinks release || true

case "$(uname -s)" in
  Linux*)
    LIB_NAME="libmktdp_driver.so"
    ;;
  Darwin*)
    LIB_NAME="libmktdp_driver.dylib"
    ;;
  MINGW*|MSYS*|CYGWIN*)
    LIB_NAME="mktdp_driver.dll"
    ;;
  *)
    echo "unsupported OS: $(uname -s)" >&2
    exit 1
    ;;
esac

if [[ ! -f "target/release/$LIB_NAME" ]]; then
  echo "expected library not found: target/release/$LIB_NAME" >&2
  exit 1
fi

echo "[2/4] Assembling dist/..."
rm -rf dist
mkdir -p dist/include
cp "target/release/$LIB_NAME" "dist/$LIB_NAME"
cp include/fingerprint.h dist/include/fingerprint.h
cp README.md dist/README.md
cp LICENSE dist/LICENSE

echo "[3/4] Release size audit..."
ls -lh "target/release/$LIB_NAME"
ls -lh dist

echo "[4/4] Done."
