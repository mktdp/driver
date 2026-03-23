//! DigitalPersona backend driver.
//!
//! This module adapts the DigitalPersona U.are.U 4500 implementation to the
//! generic multi-driver architecture.

use crate::error::Result;
use crate::usb;

/// Opaque device type for the DigitalPersona backend.
pub type Device = usb::FpDevice;

/// Image width in pixels.
pub const IMAGE_WIDTH: usize = usb::IMAGE_WIDTH;
/// Image height in pixels.
pub const IMAGE_HEIGHT: usize = usb::IMAGE_HEIGHT;
/// Raw frame header length in bytes.
pub const IMAGE_HEADER_LEN: usize = usb::IMAGE_HEADER_LEN;

/// Open a DigitalPersona-compatible scanner.
pub fn open() -> Result<Device> {
    usb::open()
}

/// Capture one raw frame from the scanner.
pub fn scan(dev: &mut Device, timeout_ms: u32) -> Result<Vec<u8>> {
    usb::scan(dev, timeout_ms)
}

/// Close the scanner device.
pub fn close(dev: Device) {
    usb::close(dev)
}
