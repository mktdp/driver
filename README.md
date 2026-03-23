# fingerprint-driver

Stateless native Rust library for capturing fingerprint images from a
**DigitalPersona U.are.U 4500** USB scanner and producing opaque biometric
templates via NIST MINDTCT + BOZORTH3.

The library exposes a **C ABI** (`extern "C"`) so any language with FFI
support (Go, PHP, Node.js, Python, …) can call it without modification.

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
| Shared library            | `target/{debug,release}/libfingerprint_driver.so` |
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

Include `fingerprint.h` and link against `libfingerprint_driver.so`.

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
gcc -o test_fp test.c -I include -L target/release -lfingerprint_driver -Wl,-rpath,'$ORIGIN'
```

---

## Go bindings (Milestone 4)

A cgo wrapper is available at `go/fingerprint` with:

- `Open() (*Device, error)`
- `Enroll(dev *Device) ([]byte, error)`
- `Verify(a, b []byte) (float64, error)`
- `MatchThreshold() float64` (default `0.06`, override via `FP_MATCH_THRESHOLD`)
- `IsMatch(score float64) bool`

Run non-hardware tests:

```bash
cd go/fingerprint
GOCACHE=/tmp/go-build go test ./...
```

Run hardware integration tests (real scanner required):

```bash
./scripts/run_go_hardware_test.sh
```

---

## Distribution packaging

Build and assemble a release bundle:

```bash
./scripts/package_dist.sh
```

This creates `dist/` with:

- `libfingerprint_driver.so` (Linux; platform-specific extension on macOS/Windows)
- `include/fingerprint.h`
- `README.md`
- `LICENSE`

---

## CI (non-hardware)

Run the same checks as CI locally:

```bash
./scripts/ci_check.sh
```

This runs formatting, clippy (`-D warnings`), Rust tests, example compile checks,
C smoke-test compile check, and Go non-hardware tests.

---

## Biometric validation

Run interactive score clustering/threshold validation (same finger vs different finger):

```bash
./scripts/run_biometric_validation.sh
```

Optional overrides:

```bash
./scripts/run_biometric_validation.sh --same 10 --diff 10 --timeout 10000
```

Capture timing tuning (optional):

```bash
FP_FINGER_DEBOUNCE_MS=220 FP_CAPTURE_SETTLE_MS=0 FP_CAPTURE_HOLD_MS=0 ./scripts/run_biometric_validation.sh --same 10 --diff 10 --timeout 10000
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
| `fp_scan_and_extract` | Yes (template buffer) | Call `fp_free(ptr, len)` exactly once on success |
| `fp_verify`           | No                    | Nothing — borrows pointers only                  |
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
│   ├── run_c_smoke.sh
│   ├── run_go_hardware_test.sh
│   ├── package_dist.sh
│   ├── ci_check.sh
│   └── run_biometric_validation.sh
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
