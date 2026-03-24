#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

echo "[1/4] Building Rust library..."
cargo build --features hardware-tests

echo "[2/4] Compiling C smoke test..."
mkdir -p target/debug
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    gcc -std=c11 -Wall -Wextra -O2 \
      -Iinclude \
      tests/test.c \
      -Ltarget/debug \
      -lmktdp_driver \
      -o target/debug/c_smoke_test.exe
    ;;
  *)
    gcc -std=c11 -Wall -Wextra -O2 \
      -Iinclude \
      tests/test.c \
      -Ltarget/debug \
      -lmktdp_driver \
      -Wl,-rpath,"$ROOT_DIR/target/debug" \
      -o target/debug/c_smoke_test
    ;;
esac

echo "[3/4] Running C smoke test..."
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    PATH="$ROOT_DIR/target/debug:$PATH" ./target/debug/c_smoke_test.exe
    ;;
  *)
    ./target/debug/c_smoke_test
    ;;
esac

echo "[4/4] Done."
