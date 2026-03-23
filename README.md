# mktdp/driver

Stateless native Rust library for capturing fingerprint images from a
**DigitalPersona U.are.U 4500** USB scanner and producing opaque biometric
templates via NIST MINDTCT + BOZORTH3.

The library exposes a **C ABI** (`extern "C"`) so any language with FFI
support (Go, PHP, Node.js, Python, …) can call it without modification.

Driver model:
- Multi-driver architecture (driver registry + runtime dispatch)
- Current backend: `digitalpersona-uru4500`
- `fp_open()` automatically selects the first available registered driver

---

## Quick start

### 1. Install system dependencies

```bash
# Fedora / RHEL / CentOS
./scripts/setup.sh

# Or manually (see script for other distros):
sudo dnf install gcc gcc-c++ cmake make pkg-config \
    libusb1-devel libstdc++-static
```

### 2. USB permissions (Linux, one-time)

```bash
sudo cp 70-fingerprint.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger
```

Then add your user to the `plugdev` group:

```bash
sudo groupadd -f plugdev
sudo usermod -aG plugdev "$USER"
# Log out and back in for the group change to take effect.
```

### 3. Build

```bash
cargo build            # debug
cargo build --release  # optimised
```

Outputs:

| Artifact                  | Path                                              |
| ------------------------- | ------------------------------------------------- |
| Shared library            | `target/{debug,release}/libmktdp_driver.so` |
| C header (auto-generated) | `include/fingerprint.h`                           |

### 4. Run unit tests (no hardware required)

```bash
cargo test
```

### 5. Run hardware integration tests (scanner must be plugged in)

```bash
cargo test --features hardware-tests
```

---

## C API overview

Include `fingerprint.h` and link against `libmktdp_driver.so`.

Key high-level functions:

- `fp_enroll_multi(...)`: capture multiple scans (recommended 6) and return one enrollment template package
- `fp_scan_and_extract(...)`: one live scan to template
- `fp_verify(...)`: compare two templates
- `fp_identify(...)`: find best match index for one probe against N stored templates
- `fp_scan_continuous(...)`: keep scanner active and stream templates via callback

```c
#include "fingerprint.h"
#include <stdio.h>

int main(void) {
    // Open scanner
    FpDevice *dev = fp_open();
    if (!dev) { fprintf(stderr, "no scanner found\n"); return 1; }

    // Scan finger → template
    uint8_t *tmpl = NULL;
    size_t   len  = 0;
    int32_t  rc   = fp_scan_and_extract(dev, 10000, &tmpl, &len);
    if (rc != FP_OK) {
        fprintf(stderr, "scan failed: %s\n", fp_strerror(rc));
        fp_close(dev);
        return 1;
    }
    printf("template: %zu bytes\n", len);

    // (enroll a second finger, then verify...)
    // double score;
    // fp_verify(tmpl_a, len_a, tmpl_b, len_b, &score);

    fp_free(tmpl, len);
    fp_close(dev);
    return 0;
}
```

Compile:

```bash
gcc -o test_fp test.c -I include -L target/release -lmktdp_driver -Wl,-rpath,'$ORIGIN'
```

Identify one probe against many enrolled templates:

```c
// probe/probe_len come from fp_scan_and_extract(...)
size_t match_index = SIZE_MAX;
double best_score = 0.0;

int32_t rc = fp_identify(
    probe, probe_len,
    templates, template_lens, candidate_count,
    0.06,
    &match_index, &best_score
);

if (rc == FP_OK && match_index != SIZE_MAX) {
    // match_index is the winning row in your application dataset
}
```

---

## Go bindings (Milestone 4)

A cgo wrapper is available at `go/fingerprint` (primarily integration/testing wrapper) with:

- `Open() (*Device, error)`
- `Enroll(dev *Device) ([]byte, error)`
- `Verify(a, b []byte) (float64, error)`
- `EnrollConsensus(dev *Device) ([]byte, error)` (6 scans of same finger -> one stable template)
- `EnrollFromScans(dev, scans, timeoutMs, maxAttemptsPerScan)` (custom enrollment flow)
- `ScanContinuously(dev, timeoutMs, onTemplate)` (attendance/check-in style continuous scanning)
- `MatchThreshold() float64` (default `0.06`, override via `FP_MATCH_THRESHOLD`)
- `IsMatch(score float64) bool`

End-user native integration walkthrough:

- `docs/END_USER_GUIDE.md`

Run non-hardware tests:

```bash
cd go/fingerprint
GOCACHE=/tmp/go-build go test ./...
```

Run hardware integration tests (real scanner required):

```bash
./tests/run_go_hardware_test.sh
```

Run 6-scan enrollment package + verify hardware test:

```bash
./tests/run_hw_enroll_merge_verify.sh
```

Run continuous scan dump (press `Ctrl+C` to stop):

```bash
./tests/run_hw_continuous_dump.sh
```

---

## Distribution packaging

Build and assemble a release bundle:

```bash
./scripts/package_dist.sh
```

This creates `dist/` with:

- `libmktdp_driver.so` (Linux; platform-specific extension on macOS/Windows)
- `include/fingerprint.h`
- `README.md`
- `LICENSE`

Automated GitHub releases are also enabled on tag push. Pushing a tag
like `v1.0.0` triggers `.github/workflows/release.yml`, which builds and
uploads a `dist` archive to the GitHub Release for that tag.

---

## CI (non-hardware)

Run the same checks as CI locally:

```bash
./scripts/ci_check.sh
```

This runs formatting, clippy (`-D warnings`), Rust tests, example compile checks,
C smoke-test compile check, and Go non-hardware tests.

Detailed test workflows (hardware + validation) are documented in
`docs/TESTING.md`.

---

## Biometric validation

Run interactive score clustering/threshold validation (same finger vs different finger):

```bash
./tests/run_biometric_validation.sh
```

Optional overrides:

```bash
./tests/run_biometric_validation.sh --same 10 --diff 10 --timeout 10000
```

Capture timing tuning (optional):

```bash
FP_FINGER_DEBOUNCE_MS=220 FP_CAPTURE_SETTLE_MS=0 FP_CAPTURE_HOLD_MS=0 ./tests/run_biometric_validation.sh --same 10 --diff 10 --timeout 10000
```

Env vars:
- `FP_MATCH_THRESHOLD`: app-level match threshold (default `0.06`)
- `FP_FINGER_DEBOUNCE_MS`: require stable finger contact for this duration before capture (default `180`)
- `FP_CAPTURE_SETTLE_MS`: extra wait after stable contact before entering capture mode (default `0`)
- `FP_CAPTURE_HOLD_MS`: wait in capture mode before bulk read (default `0`; increase only if needed)

---

## Memory contract

| Function              | Allocates?            | Caller must…                                     |
| --------------------- | --------------------- | ------------------------------------------------ |
| `fp_open`             | Yes (device handle)   | Call `fp_close` exactly once                     |
| `fp_enroll_multi`     | Yes (template buffer) | Call `fp_free(ptr, len)` exactly once on success |
| `fp_scan_and_extract` | Yes (template buffer) | Call `fp_free(ptr, len)` exactly once on success |
| `fp_verify`           | No                    | Nothing — borrows pointers only                  |
| `fp_identify`         | No                    | Nothing — borrows pointers only                  |
| `fp_scan_continuous`  | No (callback borrow)  | Copy callback bytes if you need to retain them   |
| `fp_free`             | No (deallocates)      | Never call twice on same pointer                 |
| `fp_close`            | No (deallocates)      | Never use handle after close                     |

---

## Error codes

| Constant                  | Value | Meaning                    |
| ------------------------- | ----- | -------------------------- |
| `FP_OK`                   | 0     | Success                    |
| `FP_ERR_DEVICE_NOT_FOUND` | -1    | No U.are.U 4500 on USB bus |
| `FP_ERR_USB_IO`           | -2    | USB communication error    |
| `FP_ERR_TIMEOUT`          | -3    | Timed out waiting          |
| `FP_ERR_NO_FINGER`        | -4    | No finger detected         |
| `FP_ERR_IMAGE_INVALID`    | -5    | Bad image data             |
| `FP_ERR_EXTRACT_FAIL`     | -6    | Template extraction failed |
| `FP_ERR_NULL_PTR`         | -7    | Null pointer passed to API |
| `FP_ERR_PANIC`            | -99   | Internal Rust panic caught |

Use `fp_strerror(code)` to get a human-readable string.

---

## Project structure

```
├── build.rs             # cbindgen header generation + nbis-rs lib64 fix
├── Cargo.toml
├── cbindgen.toml
├── 70-fingerprint.rules # udev rule for non-root USB access
├── include/
│   └── fingerprint.h    # auto-generated C header
├── scripts/
│   ├── setup.sh         # system dependency installer
│   ├── package_dist.sh
│   └── ci_check.sh
├── docs/
│   ├── TESTING.md       # detailed test commands and troubleshooting
│   └── END_USER_GUIDE.md # enrollment + attendance integration guide
├── tests/
│   ├── hardware.rs      # Rust hardware integration tests
│   ├── test.c           # C ABI smoke test source
│   ├── run_c_smoke.sh
│   ├── run_go_hardware_test.sh
│   ├── run_biometric_validation.sh
│   ├── run_hw_enroll_merge_verify.sh
│   └── run_hw_continuous_dump.sh
├── go/
│   └── fingerprint/     # Go cgo bindings + tests
└── src/
    ├── lib.rs           # extern "C" API surface
    ├── usb.rs           # USB device open/init/capture/close
    ├── image.rs         # deframe, descramble, normalise raw image
    ├── biometric.rs     # MINDTCT extract + BOZORTH3 verify
    └── error.rs         # FpError enum + FFI error codes
```

---

## License

MIT
