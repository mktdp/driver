//! Error types and FFI error codes for the driver library.

use thiserror::Error;

// ── FFI error codes ────────────────────────────────────────────────
// These are the constants returned across the C ABI boundary.  Every
// `extern "C"` function returns one of these.  The generated header
// re-exports them so callers in any language can compare against them.

/// Operation completed successfully.
pub const FP_OK: i32 = 0;
/// No supported fingerprint device was found on the USB bus.
pub const FP_ERR_DEVICE_NOT_FOUND: i32 = -1;
/// A USB I/O error occurred during communication with the device.
pub const FP_ERR_USB_IO: i32 = -2;
/// The operation timed out (e.g. waiting for finger presence).
pub const FP_ERR_TIMEOUT: i32 = -3;
/// No finger was detected on the sensor within the timeout window.
pub const FP_ERR_NO_FINGER: i32 = -4;
/// The captured image failed validation (wrong size or corrupted).
pub const FP_ERR_IMAGE_INVALID: i32 = -5;
/// Template extraction from the captured image failed.
pub const FP_ERR_EXTRACT_FAIL: i32 = -6;
/// A required pointer argument was null.
pub const FP_ERR_NULL_PTR: i32 = -7;
/// A Rust panic was caught at the FFI boundary.
pub const FP_ERR_PANIC: i32 = -99;

/// Internal error type used throughout the library.
///
/// Each variant maps to exactly one FFI error code so that conversion
/// to `i32` at the `extern "C"` boundary is mechanical.
#[derive(Debug, Error)]
pub enum FpError {
    #[error("no supported fingerprint device found")]
    DeviceNotFound,

    #[error("USB I/O error: {0}")]
    UsbIo(#[from] rusb::Error),

    #[error("operation timed out")]
    Timeout,

    #[error("no finger detected on sensor")]
    NoFinger,

    #[error("captured image is invalid: {0}")]
    ImageInvalid(String),

    #[error("template extraction failed: {0}")]
    ExtractFail(String),

    #[error("null pointer passed to FFI function")]
    NullPtr,
}

impl FpError {
    /// Convert to the FFI error code.
    pub fn code(&self) -> i32 {
        match self {
            FpError::DeviceNotFound => FP_ERR_DEVICE_NOT_FOUND,
            FpError::UsbIo(_) => FP_ERR_USB_IO,
            FpError::Timeout => FP_ERR_TIMEOUT,
            FpError::NoFinger => FP_ERR_NO_FINGER,
            FpError::ImageInvalid(_) => FP_ERR_IMAGE_INVALID,
            FpError::ExtractFail(_) => FP_ERR_EXTRACT_FAIL,
            FpError::NullPtr => FP_ERR_NULL_PTR,
        }
    }
}

/// Return a human-readable description for an FFI error code.
///
/// The returned slice is `'static` and null-terminated so it can be
/// handed directly across the C boundary.
pub fn strerror(code: i32) -> &'static str {
    match code {
        FP_OK => "success",
        FP_ERR_DEVICE_NOT_FOUND => "no supported fingerprint device found",
        FP_ERR_USB_IO => "USB I/O error",
        FP_ERR_TIMEOUT => "operation timed out",
        FP_ERR_NO_FINGER => "no finger detected on sensor",
        FP_ERR_IMAGE_INVALID => "captured image is invalid",
        FP_ERR_EXTRACT_FAIL => "template extraction failed",
        FP_ERR_NULL_PTR => "null pointer argument",
        FP_ERR_PANIC => "internal panic (Rust)",
        _ => "unknown error",
    }
}

/// Type alias used throughout the crate.
pub type Result<T> = std::result::Result<T, FpError>;
