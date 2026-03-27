#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

if [[ "${OS:-}" == "Windows_NT" ]]; then
  powershell -NoProfile -ExecutionPolicy Bypass -Command ". .\\scripts\\windows_common.ps1; \$cargoExe = Resolve-Cargo; Patch-NbisRsWindowsMsvc -CargoPath \$cargoExe"
fi

echo "[1/3] Building Rust shared library..."
cargo build --features hardware-tests

echo "[2/3] Running Go hardware tests..."
(
  cd go/fingerprint
  GOCACHE=/tmp/go-build FP_HARDWARE_TESTS=1 go test -v -tags hardwaretests -run TestHardware
)

echo "[3/3] Done."
