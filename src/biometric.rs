//! Biometric template extraction and verification.
//!
//! This module is the **only** code that depends on the biometric engine.
//! Currently backed by `nbis-rs` (NIST MINDTCT + BOZORTH3).  If the
//! engine is swapped later, only this file needs to change.

use crate::error::{FpError, Result};
use crate::image;
use crate::usb::{IMAGE_HEIGHT, IMAGE_WIDTH};

/// DPI of the U.are.U 4500 sensor.
const SENSOR_DPI: u32 = 500;

/// Extract a biometric template from a deframed grayscale image.
///
/// The template is an opaque byte buffer (ISO/IEC 19794-2:2005 format)
/// produced by NIST MINDTCT.  The raw pixel data is not preserved in
/// the template.
///
/// # Arguments
/// * `grayscale` — exactly `IMAGE_WIDTH × IMAGE_HEIGHT` bytes
///
/// # Returns
/// An owned `Vec<u8>` containing the serialised template.
pub fn extract(grayscale: &[u8]) -> Result<Vec<u8>> {
    use nbis::{NbisExtractor, NbisExtractorSettings};

    if grayscale.len() != IMAGE_WIDTH * IMAGE_HEIGHT {
        return Err(FpError::ImageInvalid(format!(
            "expected {} bytes, got {}",
            IMAGE_WIDTH * IMAGE_HEIGHT,
            grayscale.len()
        )));
    }

    // Encode as PNG — nbis-rs expects an encoded image, not raw pixels.
    let png_bytes = image::encode_png(grayscale, IMAGE_WIDTH as u32, IMAGE_HEIGHT as u32)?;

    let settings = NbisExtractorSettings {
        min_quality: 0.0,       // keep all minutiae
        get_center: false,      // skip ROI computation
        check_fingerprint: false, // skip SIVV check
        compute_nfiq2: false,   // skip quality scoring for speed
        ppi: Some(SENSOR_DPI as f64),
    };

    let extractor = NbisExtractor::new(settings).map_err(|e| {
        FpError::ExtractFail(format!("failed to create extractor: {}", e))
    })?;

    let minutiae = extractor.extract_minutiae(&png_bytes).map_err(|e| {
        FpError::ExtractFail(format!("minutiae extraction failed: {}", e))
    })?;

    eprintln!(
        "[extract] minutiae count: {}",
        (minutiae.to_iso_19794_2_2005().len().saturating_sub(26)) / 6
    );

    // Serialise to ISO 19794-2:2005 format.
    let template = minutiae.to_iso_19794_2_2005();

    if template.is_empty() {
        return Err(FpError::ExtractFail(
            "extracted template is empty — no minutiae found".into(),
        ));
    }

    Ok(template)
}

/// Compare two templates and return a similarity score.
///
/// Uses NIST BOZORTH3 for matching.  The raw integer score from
/// Bozorth3 is normalised to `[0.0, 1.0]` by clamping to a
/// practical maximum of 400 and dividing.
///
/// A score above ~0.1 (raw ≈ 40) indicates a match for most
/// deployments, but threshold policy is the caller's responsibility.
///
/// # Arguments
/// * `tmpl_a`, `tmpl_b` — ISO 19794-2:2005 template byte buffers
///
/// # Returns
/// A similarity score in `[0.0, 1.0]`.
pub fn verify(tmpl_a: &[u8], tmpl_b: &[u8]) -> Result<f64> {
    use nbis::{NbisExtractor, NbisExtractorSettings};

    let settings = NbisExtractorSettings {
        min_quality: 0.0,
        get_center: false,
        check_fingerprint: false,
        compute_nfiq2: false,
        ppi: Some(SENSOR_DPI as f64),
    };

    let extractor = NbisExtractor::new(settings).map_err(|e| {
        FpError::ExtractFail(format!("failed to create extractor: {}", e))
    })?;

    let m_a = extractor.load_iso_19794_2_2005(tmpl_a).map_err(|e| {
        FpError::ExtractFail(format!("failed to load template A: {}", e))
    })?;

    let m_b = extractor.load_iso_19794_2_2005(tmpl_b).map_err(|e| {
        FpError::ExtractFail(format!("failed to load template B: {}", e))
    })?;

    let raw_score = m_a.compare(&m_b);
    eprintln!("[verify] raw BOZORTH3 score: {}", raw_score);

    // Normalise: Bozorth3 returns an integer.  Scores above ~40 are
    // considered same-finger; scores can go into the hundreds.
    // We clamp to 400 and map to [0, 1].
    const MAX_SCORE: f64 = 400.0;
    let normalised = (raw_score as f64 / MAX_SCORE).clamp(0.0, 1.0);

    Ok(normalised)
}
