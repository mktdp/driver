//! Image deframing and normalisation.
//!
//! The U.are.U 4500 returns a raw frame consisting of a 64-byte
//! device header followed by pixel data.  Encryption/decryption is
//! handled by `usb::decrypt_image_data` before this module sees the
//! data — by the time `deframe` is called, the pixel bytes are
//! already plaintext.

use crate::error::{FpError, Result};
use crate::usb::{IMAGE_HEIGHT, IMAGE_HEADER_LEN, IMAGE_SIZE, IMAGE_WIDTH};

// ── Image header layout ────────────────────────────────────────────

/// Block-info flags from the device header.
mod block_flags {
    pub const NOT_PRESENT: u8 = 0x01;
}

/// Parsed image header.
struct ImageHeader {
    num_lines: u16,
    block_info: [(u8, u8); 15], // (flags, num_lines) per block
}

fn parse_header(raw: &[u8]) -> Result<ImageHeader> {
    if raw.len() < IMAGE_HEADER_LEN {
        return Err(FpError::ImageInvalid(format!(
            "header too short: {} bytes",
            raw.len()
        )));
    }

    let num_lines = u16::from_le_bytes([raw[4], raw[5]]);

    let mut block_info = [(0u8, 0u8); 15];
    for i in 0..15 {
        let offset = 16 + i * 2;
        block_info[i] = (raw[offset], raw[offset + 1]);
    }

    Ok(ImageHeader {
        num_lines,
        block_info,
    })
}

// ── Public functions ───────────────────────────────────────────────

/// Deframe and normalise a raw USB capture buffer into a flat
/// grayscale pixel array of `IMAGE_WIDTH × IMAGE_HEIGHT` bytes.
///
/// The raw buffer must already be decrypted (see
/// `usb::decrypt_image_data`).
///
/// Steps:
/// 1. Parse the 64-byte device header.
/// 2. Assemble contiguous pixel rows from block_info.
/// 3. Invert colours (sensor reports dark ridges as high values).
pub fn deframe(raw: &[u8]) -> Result<Vec<u8>> {
    let header = parse_header(raw)?;

    // Pixel data starts after the header.
    let pixel_start = IMAGE_HEADER_LEN;
    if raw.len() < pixel_start {
        return Err(FpError::ImageInvalid("buffer too short".into()));
    }

    let num_lines = header.num_lines as usize;
    if num_lines == 0 || num_lines > IMAGE_HEIGHT {
        return Err(FpError::ImageInvalid(format!(
            "invalid num_lines: {}",
            num_lines
        )));
    }

    let available = raw.len() - pixel_start;
    let needed = num_lines * IMAGE_WIDTH;
    if available < needed {
        return Err(FpError::ImageInvalid(format!(
            "pixel data too short: {} < {}",
            available, needed
        )));
    }
    let pixels = &raw[pixel_start..pixel_start + needed];

    eprintln!("[deframe] num_lines={}, pixel data={} bytes", num_lines, needed);

    // Dump block_info for diagnostics.
    for i in 0..15 {
        let (flags, count) = header.block_info[i];
        if count == 0 {
            break;
        }
        eprintln!(
            "[deframe] block {}: flags={:#04x}, lines={}",
            i, flags, count
        );
    }

    // Assemble output image from blocks (matches libfprint's
    // IMAGING_REPORT_IMAGE).  `src_row` tracks position in the
    // pixel data; it only advances for present blocks.  `dst_row`
    // always advances.
    let mut output = vec![0u8; IMAGE_SIZE];
    let mut dst_row = 0usize;
    let mut src_row = 0usize;

    for i in 0..15 {
        let (flags, count) = header.block_info[i];
        let count = count as usize;
        if count == 0 {
            break;
        }

        let src_start = src_row * IMAGE_WIDTH;
        let dst_start = dst_row * IMAGE_WIDTH;
        let bytes = count * IMAGE_WIDTH;

        if dst_start + bytes <= output.len() && src_start + bytes <= pixels.len() {
            output[dst_start..dst_start + bytes]
                .copy_from_slice(&pixels[src_start..src_start + bytes]);
        }

        if flags & block_flags::NOT_PRESENT == 0 {
            src_row += count;
        }
        dst_row += count;
    }

    // Pixel statistics for debugging.
    let (min_px, max_px, sum_px) = output.iter().fold(
        (255u8, 0u8, 0u64),
        |(mn, mx, s), &b| (mn.min(b), mx.max(b), s + b as u64),
    );
    let mean_px = sum_px / output.len() as u64;
    eprintln!(
        "[deframe] pixel stats (pre-invert): min={}, max={}, mean={}",
        min_px, max_px, mean_px
    );

    // Invert colours: sensor reports dark ridges as high values,
    // but biometric engines expect dark ridges as low values.
    for byte in output.iter_mut() {
        *byte = 255 - *byte;
    }

    Ok(output)
}

/// Encode a raw grayscale pixel buffer as PNG bytes.
///
/// `nbis-rs` expects an encoded image (PNG), not raw pixels.
/// This function wraps the grayscale data in a minimal PNG.
pub fn encode_png(
    pixels: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buf, width, height);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        // Set the DPI: 500 DPI = 500/25.4 ≈ 19685 pixels per meter
        encoder.set_pixel_dims(Some(png::PixelDimensions {
            xppu: 19685,
            yppu: 19685,
            unit: png::Unit::Meter,
        }));
        let mut writer = encoder.write_header().map_err(|e| {
            FpError::ImageInvalid(format!("PNG header write error: {}", e))
        })?;
        writer.write_image_data(pixels).map_err(|e| {
            FpError::ImageInvalid(format!("PNG data write error: {}", e))
        })?;
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_key_deterministic() {
        // The LFSR should be deterministic.
        let k1 = update_key(0x12345678);
        let k2 = update_key(0x12345678);
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_update_key_not_stuck() {
        // Running the LFSR should produce different values.
        let mut key = 0xDEADBEEF_u32;
        let start = key;
        key = update_key(key);
        assert_ne!(key, start);
    }

    #[test]
    fn test_do_decode_empty() {
        let mut data = vec![];
        let key = do_decode(&mut data, 0);
        assert_eq!(key, 0); // unchanged on empty
    }

    #[test]
    fn test_encode_png_roundtrip() {
        let pixels = vec![128u8; 10 * 10];
        let png_bytes = encode_png(&pixels, 10, 10).unwrap();
        // Should produce valid PNG (starts with PNG magic).
        assert_eq!(&png_bytes[1..4], b"PNG");
    }
}
