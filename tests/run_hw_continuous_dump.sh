#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

echo "Running continuous scan dump..."
echo "Press Ctrl+C to stop."

if [[ "${OS:-}" == "Windows_NT" ]]; then
  powershell -NoProfile -ExecutionPolicy Bypass -Command ". .\\scripts\\windows_common.ps1; \$cargoExe = Resolve-Cargo; Patch-NbisRsWindowsMsvc -CargoPath \$cargoExe"
fi

cargo run --example hw_continuous_scan_dump --features hardware-tests
