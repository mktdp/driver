#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

echo "[1/7] rustfmt check..."
cargo fmt --all --check

echo "[2/7] clippy (deny warnings)..."
cargo clippy --all-targets --features "hardware-tests,debug-logging" -- -D warnings

echo "[3/7] rust tests (non-hardware)..."
cargo test

echo "[4/7] compile hardware examples..."
cargo check --example hw_smoke_test --example biometric_validation --features hardware-tests

echo "[5/7] build debug shared library..."
cargo build --features hardware-tests

echo "[6/7] compile C smoke test..."
mkdir -p target/debug
gcc -std=c11 -Wall -Wextra -O2 \
  -Iinclude \
  tests/test.c \
  -Ltarget/debug \
  -lfingerprint_driver \
  -Wl,-rpath,"$ROOT_DIR/target/debug" \
  -o target/debug/c_smoke_test_ci

echo "[7/7] go tests (non-hardware)..."
(
  cd go/fingerprint
  GOCACHE=/tmp/go-build go test ./...
)

echo "CI checks passed."
