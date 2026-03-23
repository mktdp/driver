#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

echo "[1/2] Running 6-scan enrollment package + verify test..."
cargo run --example hw_enroll_merge_verify --features hardware-tests

echo "[2/2] Done."
