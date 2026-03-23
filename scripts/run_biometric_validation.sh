#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

echo "[1/2] Running biometric validation harness..."
cargo run --example biometric_validation --features hardware-tests -- "$@"

echo "[2/2] Done."
