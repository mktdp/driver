//! Driver abstraction layer.
//!
//! This module provides a small registry/dispatch layer so the
//! library can support multiple scanner drivers while keeping a
//! stable C ABI.
//!
//! Current status:
//! - Supported backends: DigitalPersona U.are.U 4500 (`usb` module)
//! - Future backends can be added by extending `DRIVER_REGISTRY`.

use crate::error::{FpError, Result};
use crate::image;
use crate::usb;

/// Image width in pixels for currently supported hardware.
pub const IMAGE_WIDTH: usize = usb::IMAGE_WIDTH;
/// Image height in pixels for currently supported hardware.
pub const IMAGE_HEIGHT: usize = usb::IMAGE_HEIGHT;
/// Expected raw header length in bytes.
pub const IMAGE_HEADER_LEN: usize = usb::IMAGE_HEADER_LEN;

/// Identifier for supported hardware drivers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverKind {
    /// DigitalPersona U.are.U 4500 / compatible URU4000B devices.
    DigitalPersonaUru4500,
}

impl DriverKind {
    /// Stable short name for diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            DriverKind::DigitalPersonaUru4500 => "digitalpersona-uru4500",
        }
    }
}

/// Opaque opened scanner handle.
///
/// The active backend is hidden behind an enum so the public C ABI
/// does not change as new drivers are added.
pub struct FpDevice {
    kind: DriverKind,
    inner: DeviceInner,
}

enum DeviceInner {
    DigitalPersona(usb::FpDevice),
}

struct DriverRegistration {
    kind: DriverKind,
    open: fn() -> Result<DeviceInner>,
}

const DRIVER_REGISTRY: &[DriverRegistration] = &[DriverRegistration {
    kind: DriverKind::DigitalPersonaUru4500,
    open: open_digitalpersona,
}];

/// Open the first available scanner across all registered drivers.
pub fn open() -> Result<FpDevice> {
    let mut first_non_not_found: Option<FpError> = None;

    for registration in DRIVER_REGISTRY {
        match (registration.open)() {
            Ok(inner) => {
                return Ok(FpDevice {
                    kind: registration.kind,
                    inner,
                });
            }
            Err(FpError::DeviceNotFound) => {
                // Try next registered driver.
            }
            Err(err) => {
                // Keep first concrete failure (USB I/O, timeout, etc.) in case
                // no other driver succeeds.
                if first_non_not_found.is_none() {
                    first_non_not_found = Some(err);
                }
            }
        }
    }

    Err(first_non_not_found.unwrap_or(FpError::DeviceNotFound))
}

/// Capture a raw frame from the currently active backend.
pub fn scan(dev: &mut FpDevice, timeout_ms: u32) -> Result<Vec<u8>> {
    match &mut dev.inner {
        DeviceInner::DigitalPersona(inner) => usb::scan(inner, timeout_ms),
    }
}

/// Capture and normalize one grayscale image (`IMAGE_WIDTH × IMAGE_HEIGHT`).
///
/// This is the backend-agnostic scan primitive used by the core
/// template extraction pipeline.
pub fn capture_grayscale(dev: &mut FpDevice, timeout_ms: u32) -> Result<Vec<u8>> {
    match &mut dev.inner {
        DeviceInner::DigitalPersona(inner) => {
            let raw = usb::scan(inner, timeout_ms)?;
            image::deframe(&raw)
        }
    }
}

/// Close an opened scanner handle.
pub fn close(dev: FpDevice) {
    match dev.inner {
        DeviceInner::DigitalPersona(inner) => usb::close(inner),
    }
}

/// Return the active backend short name.
pub fn active_driver_name(dev: &FpDevice) -> &'static str {
    dev.kind.as_str()
}

/// Return all registered backend names.
pub fn supported_driver_names() -> &'static [&'static str] {
    const NAMES: &[&str] = &["digitalpersona-uru4500"];
    NAMES
}

fn open_digitalpersona() -> Result<DeviceInner> {
    usb::open().map(DeviceInner::DigitalPersona)
}

#[cfg(test)]
mod tests {
    use super::{supported_driver_names, DriverKind};

    #[test]
    fn driver_kind_name_is_stable() {
        assert_eq!(
            DriverKind::DigitalPersonaUru4500.as_str(),
            "digitalpersona-uru4500"
        );
    }

    #[test]
    fn supported_driver_registry_is_non_empty() {
        assert!(!supported_driver_names().is_empty());
    }
}
