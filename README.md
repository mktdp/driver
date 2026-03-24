# mktdp/driver

Stateless native Rust fingerprint driver library for DigitalPersona U.are.U 4500.

It does two jobs:
- capture a fingerprint image from USB
- extract and compare templates through `nbis-rs` (MINDTCT + BOZORTH3)

The public interface is a C ABI (`extern "C"`), so any FFI-capable language can call it.

## Quick Start

### 1) Install prerequisites

Linux:

```bash
./scripts/setup.sh
```

Windows 11 (PowerShell):

```powershell
.\scripts\setup_windows.ps1
```

Windows notes:
- Device must be bound to WinUSB (VID `05BA`, PID `000A`) before `fp_open()` can talk to it.
- Zadig is one way to do that binding.
- Replug after driver changes.
- MSYS2 MinGW (`C:\msys64\mingw64\bin`) is recommended for Go/cgo workflows.

### 2) Linux USB permissions (one-time)

```bash
sudo cp 70-fingerprint.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger
sudo groupadd -f plugdev
sudo usermod -aG plugdev "$USER"
```

Log out and back in after group changes.

### 3) Build

```bash
cargo build
cargo build --release
```

Output artifacts:
- Linux: `target/{debug,release}/libmktdp_driver.so`
- macOS: `target/{debug,release}/libmktdp_driver.dylib`
- Windows: `target/{debug,release}/mktdp_driver.dll`
- Generated header: `include/fingerprint.h`

## C API At A Glance

Core exported operations:
- `fp_open` / `fp_close`
- `fp_enroll_multi`
- `fp_scan_and_extract`
- `fp_verify`
- `fp_identify`
- `fp_scan_continuous`
- `fp_free`
- `fp_strerror`

See `include/fingerprint.h` for signatures and constants.

Minimal C flow:

```c
#include "fingerprint.h"
#include <stdio.h>

int main(void) {
    FpDevice *dev = fp_open();
    if (!dev) return 1;

    uint8_t *tmpl = NULL;
    size_t len = 0;
    int32_t rc = fp_scan_and_extract(dev, 10000, &tmpl, &len);
    if (rc != FP_OK) {
        fprintf(stderr, "scan failed: %s\n", fp_strerror(rc));
        fp_close(dev);
        return 1;
    }

    fp_free(tmpl, len);
    fp_close(dev);
    return 0;
}
```

## Go Wrapper

`go/fingerprint` provides a cgo wrapper used by tests and integrations.

Non-hardware tests:

```bash
cd go/fingerprint
go test ./...
```

Hardware tests:

```bash
./tests/run_go_hardware_test.sh
```

```powershell
.\tests\run_go_hardware_test.ps1
```

On Windows, `run_go_hardware_test.ps1` auto-enables cgo with MSYS2 MinGW when available.

## Packaging

Build and assemble `dist/`:

```bash
./scripts/package_dist.sh
```

```powershell
.\scripts\package_dist.ps1
```

`dist/` contains:
- shared library (`.so`, `.dylib`, or `.dll`)
- `include/fingerprint.h`
- `README.md`
- `LICENSE`

## Testing

Run CI-equivalent local checks:

```bash
./scripts/ci_check.sh
```

```powershell
.\scripts\ci_check.ps1
```

Hardware helpers:
- `tests/run_c_smoke.{sh,ps1}`
- `tests/run_hw_enroll_merge_verify.{sh,ps1}`
- `tests/run_hw_continuous_dump.{sh,ps1}`
- `tests/run_biometric_validation.{sh,ps1}`

More detail:
- `docs/TESTING.md`
- `docs/END_USER_GUIDE.md`

## Memory Ownership Contract

- Buffers returned from `fp_scan_and_extract` and `fp_enroll_multi` must be released exactly once with `fp_free(ptr, len)`.
- Device handles returned by `fp_open` must be closed with `fp_close`.
- `fp_verify` and `fp_identify` borrow caller memory; they do not allocate output buffers.

## License

MIT
