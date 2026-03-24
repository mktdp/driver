# Roadmap

## Completed

### Milestone 1 — USB communication (core driver)
- [x] USB device enumeration by VID/PID (`0x05ba:0x000a`)
- [x] Kernel driver detach + interface claim
- [x] Device init state-machine (hwstat, power cycle, IRQDATA_SCANPWR_ON)
- [x] Finger presence detection via interrupt endpoint polling
- [x] Raw image capture via bulk-in endpoint
- [x] Bulk-read retry logic (device sends ZLP on first read after MODE_CAPTURE)
- [x] Finger-off detection (`MODE_AWAIT_FINGER_OFF`) between scans
- [x] Image deframing (64-byte header parsing, block assembly)
- [x] LFSR descrambling via device key exchange (`REG_SCRAMBLE_DATA_INDEX`/`REG_SCRAMBLE_DATA_KEY`)
- [x] Colour inversion (sensor convention → biometric convention)

### Milestone 2 — Template extraction
- [x] PNG encoding of raw grayscale buffer (nbis-rs expects encoded image)
- [x] MINDTCT minutiae extraction via nbis-rs
- [x] ISO 19794-2:2005 template serialisation
- [x] BOZORTH3 template matching with score normalisation

### Milestone 3 — Library + C ABI
- [x] `extern "C"` API: `fp_open`, `fp_scan_and_extract`, `fp_verify`, `fp_free`, `fp_close`, `fp_strerror`
- [x] `std::panic::catch_unwind` on every FFI entry point
- [x] Null-pointer checks at every FFI entry point
- [x] Memory ownership via `Vec::leak` / `Vec::from_raw_parts`
- [x] `cbindgen` auto-generates `include/fingerprint.h`
- [x] Error codes + `FpError` enum with `thiserror`
- [x] Build system: `cdylib` + `rlib` crate types
- [x] nbis-rs `lib64 → lib` symlink fix in `build.rs`
- [x] udev rules for non-root USB access
- [x] Setup script for system dependencies

### Hardware smoke test
- [x] Plug in U.are.U 4500, run `cargo run --example hw_smoke_test --features hardware-tests` end-to-end
- [x] Validated: two scans, same finger, BOZORTH3 raw score=67 → MATCH
- [x] Validated: MINDTCT produces 28–88 minutiae, templates 194–554 bytes
- [x] Validated: decrypted images show clear fingerprint ridges at 500 DPI

---

## In progress

### Code cleanup
- [x] Remove `eprintln!` debug logging (gated behind `debug-logging` feature flag)
- [x] Add image V+H flip for DP_URU4000B (libfprint sets `FPI_IMAGE_V_FLIPPED | FPI_IMAGE_H_FLIPPED`)
- [x] Run `cargo clippy --deny warnings` and fix any issues

### Biometric engine validation
- [x] Enroll same finger 10× and verify all pairs (score clustering test)
- [x] Enroll different fingers and verify cross-pairs stay below threshold
- [x] Tune match threshold — current empirical recommendation: raw ≈20 (normalised 0.05), measured FRR 12.22% / FAR 1.00% on 10×10 local dataset

---

## Next steps

### Milestone 3b — C smoke test
- [x] Write `tests/test.c` that calls `fp_open` → `fp_scan_and_extract` ×2 → `fp_verify` → `fp_free` ×2 → `fp_close`
- [x] Makefile or shell script to compile and run the C test

### Milestone 4 — Go bindings
- [x] Go package wrapping the C ABI: `Enroll(dev *Device) ([]byte, error)`, `Verify(a, b []byte) (float64, error)`
- [x] Go integration test: enroll → verify → print score
- [x] Confirm `Verify` works from a goroutine without a device handle

### Milestone 5 — Distribution packaging
- [x] `cargo build --release` produces optimised `.so`
- [x] `dist/` directory: `libmktdp_driver.so`, `fingerprint.h`, `LICENSE`, `README.md`
- [x] Release build size audit (strip debug symbols)

### Milestone 6
- [x] CI pipeline (unit tests without hardware, integration tests gated behind `hardware-tests` feature)
- [x] Windows build + test
- [ ] macOS build + test

### Future considerations (out of scope for now)
- [ ] Template format versioning / migration strategy
- [ ] Benchmark: extraction + matching latency on target hardware
- [ ] If nbis-rs accuracy is poor, evaluate NBIS via direct C FFI
