//! Fingerprint driver — C-compatible public API.
//!
//! This crate provides a stateless library for capturing fingerprint
//! images from DigitalPersona U.are.U 4500 (and compatible) USB
//! scanners and producing opaque biometric templates.
//!
//! The library exposes a C ABI (`extern "C"`) so it can be called
//! from any language with FFI support.  It is compiled as a shared
//! library (`.so`, `.dylib`, or `.dll`).
//!
//! # Memory ownership
//!
//! - [`fp_scan_and_extract`] allocates a template buffer on the Rust
//!   heap and transfers ownership to the caller.
//! - The caller **must** call [`fp_free`] exactly once per successful
//!   extraction to release that buffer.
//! - [`fp_verify`] does **not** allocate — it borrows the pointers
//!   for the duration of the call only.

pub mod biometric;
pub mod error;
pub mod image;
pub mod usb;

use std::ffi::CStr;
use std::panic;
use std::ptr;

use error::*;

/// Opaque device handle exposed to C callers.
///
/// The caller receives a raw pointer from [`fp_open`] and must pass
/// it back to [`fp_scan_and_extract`] and [`fp_close`].
pub type FpDevice = usb::FpDevice;

// ─── fp_open ───────────────────────────────────────────────────────

/// Open the first available U.are.U 4500 scanner.
///
/// Returns an opaque device handle on success, or `NULL` on failure.
/// The handle is **not** thread-safe.
#[no_mangle]
pub extern "C" fn fp_open() -> *mut FpDevice {
    let result = panic::catch_unwind(|| match usb::open() {
        Ok(dev) => Box::into_raw(Box::new(dev)),
        Err(_) => ptr::null_mut(),
    });
    match result {
        Ok(ptr) => ptr,
        Err(_) => ptr::null_mut(),
    }
}

// ─── fp_scan_and_extract ───────────────────────────────────────────

/// Capture a fingerprint and extract a template.
///
/// Blocks until a finger is placed on the sensor (or `timeout_ms`
/// expires), captures the image, and writes the template to
/// `*template_out` / `*len_out`.
///
/// On success: returns `FP_OK`, `*template_out` points to a
/// heap-allocated buffer, `*len_out` is its length.  Caller **must**
/// call `fp_free(*template_out, *len_out)`.
///
/// On failure: returns a non-zero error code.  `*template_out` and
/// `*len_out` are set to `NULL` / `0`.
#[no_mangle]
pub extern "C" fn fp_scan_and_extract(
    dev: *mut FpDevice,
    timeout_ms: u32,
    template_out: *mut *mut u8,
    len_out: *mut usize,
) -> i32 {
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        // Null-pointer checks.
        if dev.is_null() || template_out.is_null() || len_out.is_null() {
            return FP_ERR_NULL_PTR;
        }

        // SAFETY: `dev` was allocated by `fp_open` via `Box::into_raw`
        // and the caller guarantees it has not been freed. We borrow it
        // mutably for the duration of this call.
        let dev_ref = unsafe { &mut *dev };

        // Zero outputs before attempting anything.
        unsafe {
            *template_out = ptr::null_mut();
            *len_out = 0;
        }

        // 1. Scan (wait for finger + capture image).
        let raw_frame = match usb::scan(dev_ref, timeout_ms) {
            Ok(frame) => frame,
            Err(e) => return e.code(),
        };

        // 2. Deframe and normalise.
        let grayscale = match image::deframe(&raw_frame) {
            Ok(pixels) => pixels,
            Err(e) => return e.code(),
        };
        // `raw_frame` is dropped here — raw pixel data is ephemeral.

        // 3. Extract template.
        let template = match biometric::extract(&grayscale) {
            Ok(t) => t,
            Err(e) => return e.code(),
        };
        // `grayscale` is dropped here.

        // 4. Transfer ownership of the template buffer to the caller.
        let len = template.len();
        let ptr = template.leak().as_mut_ptr();

        unsafe {
            *template_out = ptr;
            *len_out = len;
        }

        FP_OK
    }));

    match result {
        Ok(code) => code,
        Err(_) => FP_ERR_PANIC,
    }
}

// ─── fp_verify ─────────────────────────────────────────────────────

/// Compare two templates and output a similarity score.
///
/// Does **not** require a device handle.  `score_out` is written on
/// success (range `[0.0, 1.0]`).
///
/// Returns `FP_OK` on success, non-zero on failure.
#[no_mangle]
pub extern "C" fn fp_verify(
    tmpl_a: *const u8,
    len_a: usize,
    tmpl_b: *const u8,
    len_b: usize,
    score_out: *mut f64,
) -> i32 {
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        if tmpl_a.is_null() || tmpl_b.is_null() || score_out.is_null() {
            return FP_ERR_NULL_PTR;
        }

        // SAFETY: the caller guarantees that `tmpl_a` points to at
        // least `len_a` readable bytes, and similarly for `tmpl_b`.
        let a = unsafe { std::slice::from_raw_parts(tmpl_a, len_a) };
        let b = unsafe { std::slice::from_raw_parts(tmpl_b, len_b) };

        match biometric::verify(a, b) {
            Ok(score) => {
                unsafe { *score_out = score };
                FP_OK
            }
            Err(e) => e.code(),
        }
    }));

    match result {
        Ok(code) => code,
        Err(_) => FP_ERR_PANIC,
    }
}

// ─── fp_free ───────────────────────────────────────────────────────

/// Free a template buffer previously returned by `fp_scan_and_extract`.
///
/// # Safety
///
/// `ptr` must have been returned by a successful call to
/// `fp_scan_and_extract`.  Passing any other pointer is UB.
/// Calling `fp_free` twice on the same pointer is UB.
#[no_mangle]
pub extern "C" fn fp_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() || len == 0 {
        return;
    }
    // SAFETY: `ptr` was produced by `Vec::leak()` in
    // `fp_scan_and_extract`.  We reconstruct the Vec here so it is
    // dropped (deallocated).  The capacity was equal to len at the
    // time of leak.
    let _ = unsafe { Vec::from_raw_parts(ptr, len, len) };
}

// ─── fp_close ──────────────────────────────────────────────────────

/// Close the device handle returned by `fp_open`.
///
/// After this call the pointer is invalid.
#[no_mangle]
pub extern "C" fn fp_close(dev: *mut FpDevice) {
    if dev.is_null() {
        return;
    }
    // SAFETY: `dev` was allocated by `Box::into_raw` in `fp_open`.
    let device = unsafe { Box::from_raw(dev) };
    usb::close(*device);
}

// ─── fp_strerror ───────────────────────────────────────────────────

/// Return a static, null-terminated human-readable string for an error code.
///
/// The returned pointer is valid for the lifetime of the process.
/// Returns `"unknown error"` for unrecognised codes.
#[no_mangle]
pub extern "C" fn fp_strerror(code: i32) -> *const std::os::raw::c_char {
    // All strings returned by `error::strerror` are compile-time
    // literals, so they live in static memory and are implicitly
    // null-terminated when represented as C strings.
    let s = error::strerror(code);
    // We use a set of pre-built CStr constants so the pointer is
    // truly 'static and null-terminated.
    static STRINGS: &[(&str, &CStr)] = &[
        ("success", c"success"),
        ("no supported fingerprint device found", c"no supported fingerprint device found"),
        ("USB I/O error", c"USB I/O error"),
        ("operation timed out", c"operation timed out"),
        ("no finger detected on sensor", c"no finger detected on sensor"),
        ("captured image is invalid", c"captured image is invalid"),
        ("template extraction failed", c"template extraction failed"),
        ("null pointer argument", c"null pointer argument"),
        ("internal panic (Rust)", c"internal panic (Rust)"),
        ("unknown error", c"unknown error"),
    ];

    for (key, cstr) in STRINGS {
        if *key == s {
            return cstr.as_ptr();
        }
    }

    c"unknown error".as_ptr()
}
