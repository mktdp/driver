//! Fingerprint driver — C-compatible public API.
//!
//! This crate provides a stateless library for capturing fingerprint
//! images from supported USB scanner drivers and producing opaque
//! biometric templates.
//!
//! The backend is driver-pluggable via [`crate::driver`].  Currently
//! shipped backend: DigitalPersona U.are.U 4500.
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
pub mod driver;
pub mod error;
pub mod image;
pub mod usb;

use std::ffi::CStr;
use std::os::raw::c_void;
use std::panic;
use std::ptr;
use std::time::Duration;

use error::*;

/// Opaque device handle exposed to C callers.
///
/// The caller receives a raw pointer from [`fp_open`] and must pass
/// it back to [`fp_scan_and_extract`] and [`fp_close`].
pub type FpDevice = driver::FpDevice;
/// Callback signature used by [`fp_scan_continuous`].
///
/// The callback receives a borrowed template pointer valid only for
/// the duration of the callback invocation. Return `true` to keep
/// scanning, or `false` to stop.
pub type FpTemplateCallback = unsafe extern "C" fn(*const u8, usize, *mut c_void) -> bool;

// ─── fp_open ───────────────────────────────────────────────────────

/// Open the first available supported scanner.
///
/// Returns an opaque device handle on success, or `NULL` on failure.
/// The handle is **not** thread-safe.
#[no_mangle]
pub extern "C" fn fp_open() -> *mut FpDevice {
    let result = panic::catch_unwind(|| match driver::open() {
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
#[allow(clippy::not_unsafe_ptr_arg_deref)]
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

        // Scan and extract template.
        let template = match scan_and_extract_template(dev_ref, timeout_ms) {
            Ok(t) => t,
            Err(e) => return e.code(),
        };

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

// ─── fp_enroll_multi ──────────────────────────────────────────────

/// Capture multiple scans of the same finger and combine them into one
/// enrollment template package.
///
/// This function performs enrollment-style multi-capture:
/// - captures `scan_count` templates from the same finger
/// - retries each slot up to `max_attempts_per_scan` on recoverable capture failures
/// - combines all captured templates into one opaque multi-view template
///
/// `scan_count` should typically be `6`.
///
/// On success: returns `FP_OK`, sets `*template_out` and `*len_out`.
/// Caller must free with `fp_free`.
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn fp_enroll_multi(
    dev: *mut FpDevice,
    timeout_ms: u32,
    scan_count: u32,
    max_attempts_per_scan: u32,
    template_out: *mut *mut u8,
    len_out: *mut usize,
) -> i32 {
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        if dev.is_null() || template_out.is_null() || len_out.is_null() {
            return FP_ERR_NULL_PTR;
        }
        if scan_count < 2 {
            return FP_ERR_EXTRACT_FAIL;
        }
        if max_attempts_per_scan == 0 {
            return FP_ERR_EXTRACT_FAIL;
        }

        // SAFETY: `dev` is owned by caller and valid for this call.
        let dev_ref = unsafe { &mut *dev };

        unsafe {
            *template_out = ptr::null_mut();
            *len_out = 0;
        }

        let mut captured = Vec::with_capacity(scan_count as usize);
        for idx in 0..scan_count {
            let mut last_err: Option<FpError> = None;
            let mut success = None;

            for _attempt in 0..max_attempts_per_scan {
                match scan_and_extract_template(dev_ref, timeout_ms) {
                    Ok(t) => {
                        success = Some(t);
                        last_err = None;
                        break;
                    }
                    Err(e) => {
                        let recoverable = is_recoverable_capture_error(&e);
                        last_err = Some(e);
                        if recoverable {
                            std::thread::sleep(Duration::from_millis(900));
                            continue;
                        }
                        return last_err
                            .as_ref()
                            .map(|err| err.code())
                            .unwrap_or(FP_ERR_EXTRACT_FAIL);
                    }
                }
            }

            let template = match success {
                Some(t) => t,
                None => return last_err.map(|e| e.code()).unwrap_or(FP_ERR_EXTRACT_FAIL),
            };

            captured.push(template);

            // Give the user a short interval to lift/re-place between captures.
            if idx + 1 < scan_count {
                std::thread::sleep(Duration::from_millis(1200));
            }
        }

        let merged = match biometric::enrollment_bundle(&captured) {
            Ok(t) => t,
            Err(e) => return e.code(),
        };

        let len = merged.len();
        let ptr = merged.leak().as_mut_ptr();
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
#[allow(clippy::not_unsafe_ptr_arg_deref)]
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

// ─── fp_identify ───────────────────────────────────────────────────

/// Identify the best match for one probe template against a candidate set.
///
/// This is a 1:N helper for backend/app code that already has a table of
/// stored enrollment templates (for example, users in an access-control app).
///
/// Inputs:
/// - `probe_tmpl` / `probe_len`: the freshly scanned template
/// - `candidates`: array of candidate template pointers
/// - `candidate_lens`: array of candidate template lengths
/// - `candidate_count`: number of entries in both arrays
/// - `threshold`: match threshold in `[0.0, 1.0]`
///
/// Outputs:
/// - `*match_index_out`: index of best candidate if score >= threshold, else `SIZE_MAX`
/// - `*match_score_out`: best score found across all candidates
///
/// Returns `FP_OK` on success, non-zero on failure.
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn fp_identify(
    probe_tmpl: *const u8,
    probe_len: usize,
    candidates: *const *const u8,
    candidate_lens: *const usize,
    candidate_count: usize,
    threshold: f64,
    match_index_out: *mut usize,
    match_score_out: *mut f64,
) -> i32 {
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        if probe_tmpl.is_null() || match_index_out.is_null() || match_score_out.is_null() {
            return FP_ERR_NULL_PTR;
        }
        if !(0.0..=1.0).contains(&threshold) {
            return FP_ERR_EXTRACT_FAIL;
        }

        // Default output: no match, zero score.
        unsafe {
            *match_index_out = usize::MAX;
            *match_score_out = 0.0;
        }

        if candidate_count == 0 {
            return FP_OK;
        }
        if candidates.is_null() || candidate_lens.is_null() {
            return FP_ERR_NULL_PTR;
        }

        // SAFETY: caller guarantees probe pointer is valid for `probe_len`.
        let probe = unsafe { std::slice::from_raw_parts(probe_tmpl, probe_len) };
        // SAFETY: caller guarantees these arrays are valid for `candidate_count`.
        let candidate_ptrs = unsafe { std::slice::from_raw_parts(candidates, candidate_count) };
        // SAFETY: caller guarantees these arrays are valid for `candidate_count`.
        let lens = unsafe { std::slice::from_raw_parts(candidate_lens, candidate_count) };

        let mut scored = Vec::with_capacity(candidate_count);
        for (idx, (&cand_ptr, &cand_len)) in candidate_ptrs.iter().zip(lens.iter()).enumerate() {
            if cand_len == 0 {
                continue;
            }
            if cand_ptr.is_null() {
                return FP_ERR_NULL_PTR;
            }
            // SAFETY: caller guarantees each candidate pointer is valid for its length.
            let candidate = unsafe { std::slice::from_raw_parts(cand_ptr, cand_len) };
            let score = match biometric::verify(probe, candidate) {
                Ok(s) => s,
                Err(e) => return e.code(),
            };
            scored.push((idx, score));
        }

        let (match_idx, best_score) = choose_best_match(scored, threshold);
        unsafe {
            *match_index_out = match_idx;
            *match_score_out = best_score;
        }
        FP_OK
    }));

    match result {
        Ok(code) => code,
        Err(_) => FP_ERR_PANIC,
    }
}

// ─── fp_scan_continuous ───────────────────────────────────────────

/// Keep scanner active and stream templates via callback.
///
/// This helper is intended for attendance/check-in workflows where
/// the scanner stays open and many users scan one after another.
///
/// - `max_scans = 0` means run until callback returns `false`.
/// - Recoverable capture failures are ignored and scanning continues.
/// - Callback receives a borrowed template pointer that is only valid
///   during callback execution; copy it if needed.
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn fp_scan_continuous(
    dev: *mut FpDevice,
    timeout_ms: u32,
    max_scans: u32,
    callback: FpTemplateCallback,
    user_data: *mut c_void,
) -> i32 {
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        if dev.is_null() {
            return FP_ERR_NULL_PTR;
        }

        // SAFETY: `dev` was allocated by `fp_open` and is valid for this call.
        let dev_ref = unsafe { &mut *dev };

        let mut emitted: u32 = 0;
        loop {
            if max_scans != 0 && emitted >= max_scans {
                return FP_OK;
            }

            match scan_and_extract_template(dev_ref, timeout_ms) {
                Ok(template) => {
                    emitted = emitted.saturating_add(1);
                    // SAFETY: callback is provided by caller and must follow the
                    // documented ABI contract. Pointers are valid for this call.
                    let keep_scanning =
                        unsafe { callback(template.as_ptr(), template.len(), user_data) };
                    if !keep_scanning {
                        return FP_OK;
                    }
                }
                Err(e) => {
                    if is_recoverable_capture_error(&e) {
                        std::thread::sleep(Duration::from_millis(250));
                        continue;
                    }
                    return e.code();
                }
            }
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
#[allow(clippy::not_unsafe_ptr_arg_deref)]
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
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn fp_close(dev: *mut FpDevice) {
    if dev.is_null() {
        return;
    }
    // SAFETY: `dev` was allocated by `Box::into_raw` in `fp_open`.
    let device = unsafe { Box::from_raw(dev) };
    driver::close(*device);
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
        (
            "no supported fingerprint device found",
            c"no supported fingerprint device found",
        ),
        ("USB I/O error", c"USB I/O error"),
        ("operation timed out", c"operation timed out"),
        (
            "no finger detected on sensor",
            c"no finger detected on sensor",
        ),
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

fn scan_and_extract_template(dev_ref: &mut FpDevice, timeout_ms: u32) -> error::Result<Vec<u8>> {
    // 1. Capture + normalise grayscale image through active backend.
    let grayscale = driver::capture_grayscale(dev_ref, timeout_ms)?;

    // 2. Extract template.
    let template = biometric::extract(&grayscale)?;
    // `grayscale` is dropped here.

    Ok(template)
}

fn choose_best_match<I>(scores: I, threshold: f64) -> (usize, f64)
where
    I: IntoIterator<Item = (usize, f64)>,
{
    let mut best_idx = usize::MAX;
    let mut best_score = 0.0_f64;

    for (idx, score) in scores {
        if score > best_score {
            best_score = score;
            best_idx = idx;
        }
    }

    if best_score < threshold {
        (usize::MAX, best_score)
    } else {
        (best_idx, best_score)
    }
}

fn is_recoverable_capture_error(err: &FpError) -> bool {
    matches!(
        err,
        FpError::Timeout | FpError::NoFinger | FpError::ImageInvalid(_) | FpError::ExtractFail(_)
    )
}

#[cfg(test)]
mod tests {
    use super::choose_best_match;

    #[test]
    fn choose_best_match_returns_best_index_when_above_threshold() {
        let (idx, score) = choose_best_match([(0, 0.12), (1, 0.35), (2, 0.20)], 0.25);
        assert_eq!(idx, 1);
        assert!((score - 0.35).abs() < f64::EPSILON);
    }

    #[test]
    fn choose_best_match_returns_no_match_when_below_threshold() {
        let (idx, score) = choose_best_match([(0, 0.02), (1, 0.05), (2, 0.01)], 0.06);
        assert_eq!(idx, usize::MAX);
        assert!((score - 0.05).abs() < f64::EPSILON);
    }

    #[test]
    fn choose_best_match_handles_empty_input() {
        let (idx, score) = choose_best_match(std::iter::empty::<(usize, f64)>(), 0.1);
        assert_eq!(idx, usize::MAX);
        assert!((score - 0.0).abs() < f64::EPSILON);
    }
}
