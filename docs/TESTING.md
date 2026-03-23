# Testing Guide

This project keeps test artifacts in `tests/` and language-level tests in their
native locations (`tests/*.rs`, `go/fingerprint/*_test.go`).

## Quick Commands

Non-hardware checks:

```bash
cargo test
./scripts/ci_check.sh
```

Rust hardware smoke test:

```bash
cargo run --example hw_smoke_test --features hardware-tests
```

C ABI hardware smoke test:

```bash
./tests/run_c_smoke.sh
```

Go hardware tests:

```bash
./tests/run_go_hardware_test.sh
```

6-scan enroll package + verify test:

```bash
./tests/run_hw_enroll_merge_verify.sh
```

Continuous scan dump (stop with `Ctrl+C`):

```bash
./tests/run_hw_continuous_dump.sh
```

Biometric validation harness:

```bash
./tests/run_biometric_validation.sh --same 10 --diff 10 --timeout 10000
```

## Useful Environment Variables

Match policy:

- `FP_MATCH_THRESHOLD`: default match threshold (default `0.06`)

Capture timing:

- `FP_FINGER_DEBOUNCE_MS`: stable contact time before capture (default `180`)
- `FP_CAPTURE_SETTLE_MS`: delay after stable contact before capture mode (default `0`)
- `FP_CAPTURE_HOLD_MS`: wait in capture mode before bulk read (default `0`)

Example:

```bash
FP_FINGER_DEBOUNCE_MS=220 FP_CAPTURE_SETTLE_MS=0 FP_CAPTURE_HOLD_MS=0 ./tests/run_biometric_validation.sh --same 10 --diff 10 --timeout 10000
```

## Troubleshooting

- If scanner open fails on Linux, re-check `70-fingerprint.rules` and group membership (`plugdev`).
- If templates are very small or extraction fails intermittently, rerun biometric validation with retries and inspect debug images under `storage/`.
- If release build fails with missing `nfiq2`, rerun `./scripts/package_dist.sh` (it includes the `lib64 -> lib` fallback fix).
