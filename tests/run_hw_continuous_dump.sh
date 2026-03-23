#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

echo "Running continuous scan dump..."
echo "Press Ctrl+C to stop."
cargo run --example hw_continuous_scan_dump --features hardware-tests
