//! Biometric template extraction and verification.
//!
//! This module is the **only** code that depends on the biometric engine.
//! Currently backed by `nbis-rs` (NIST MINDTCT + BOZORTH3).  If the
//! engine is swapped later, only this file needs to change.

use crate::error::{FpError, Result};
use crate::image;
use crate::usb::{IMAGE_HEIGHT, IMAGE_WIDTH};
use std::f64::consts::PI;

macro_rules! debug_log {
    ($($arg:tt)*) => {
        if cfg!(feature = "debug-logging") {
            eprintln!($($arg)*);
        }
    };
}

/// DPI of the U.are.U 4500 sensor.
const SENSOR_DPI: u32 = 500;
/// ISO/IEC 19794-2:2005 fixed template header size in bytes.
const ISO_TEMPLATE_HEADER_LEN: usize = 26;
/// Minimum valid ISO template size (header + at least one minutia).
const ISO_TEMPLATE_MIN_LEN: usize = 28;
/// Magic bytes for ISO/IEC 19794-2:2005 template header.
const ISO_TEMPLATE_MAGIC: &[u8; 8] = b"FMR\0 20\0";
/// Max minutiae retained by NBIS bozorth data structures.
const MAX_BOZORTH_MINUTIAE: usize = 150;
/// Practical cap for merged enrollment templates.
///
/// We keep this lower than the absolute Bozorth cap to avoid packing
/// too many low-stability minutiae from multi-scan fusion.
const MAX_ENROLLMENT_MINUTIAE: usize = 90;
/// Spatial tolerance (pixels) for grouping minutiae across scans.
const MERGE_CLUSTER_TOLERANCE_PX: i32 = 12;
/// Angular tolerance (ISO code units, 0..255) for grouping minutiae.
const MERGE_CLUSTER_TOLERANCE_ANGLE: i32 = 14; // ~19.7 degrees
/// If support filtering yields very few minutiae, fall back to ranked
/// full set to avoid an overly sparse enrollment template.
const MIN_STABLE_MINUTIAE_TARGET: usize = 24;
/// Opaque multi-view enrollment bundle magic.
const TEMPLATE_BUNDLE_MAGIC: &[u8; 4] = b"FPM1";
/// Opaque multi-view enrollment bundle version.
const TEMPLATE_BUNDLE_VERSION: u8 = 1;
/// Maximum number of view templates in one enrollment bundle.
const TEMPLATE_BUNDLE_MAX_VIEWS: usize = 32;

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
        min_quality: 0.0,         // keep all minutiae
        get_center: false,        // skip ROI computation
        check_fingerprint: false, // skip SIVV check
        compute_nfiq2: false,     // skip quality scoring for speed
        ppi: Some(SENSOR_DPI as f64),
    };

    let extractor = NbisExtractor::new(settings)
        .map_err(|e| FpError::ExtractFail(format!("failed to create extractor: {}", e)))?;

    let minutiae = extractor
        .extract_minutiae(&png_bytes)
        .map_err(|e| FpError::ExtractFail(format!("minutiae extraction failed: {}", e)))?;

    // Serialise to ISO 19794-2:2005 format.
    let template = minutiae.to_iso_19794_2_2005();
    let minutiae_count = template.len().saturating_sub(ISO_TEMPLATE_HEADER_LEN) / 6;

    debug_log!("[extract] minutiae count: {}", minutiae_count);

    if template.is_empty() || minutiae_count == 0 {
        return Err(FpError::ExtractFail(
            "extracted template has no minutiae".into(),
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
///
/// For multi-view enrollment bundles (`FPM1`), the final score is the
/// median of all pairwise view scores (consensus scoring).
pub fn verify(tmpl_a: &[u8], tmpl_b: &[u8]) -> Result<f64> {
    let extractor = new_extractor()?;
    verify_with_extractor(&extractor, tmpl_a, tmpl_b)
}

/// Identify the best candidate for one probe template.
///
/// Returns `(best_index, best_score)` over `candidates`.
pub fn identify_best(probe: &[u8], candidates: &[&[u8]]) -> Result<(usize, f64)> {
    if candidates.is_empty() {
        return Err(FpError::ExtractFail("no candidate templates".into()));
    }

    let extractor = new_extractor()?;

    let mut best_index = 0usize;
    let mut best_score = f64::MIN;
    for (idx, candidate) in candidates.iter().enumerate() {
        let score = verify_with_extractor(&extractor, probe, candidate)?;
        if score > best_score {
            best_score = score;
            best_index = idx;
        }
    }

    Ok((best_index, best_score.max(0.0)))
}

/// Build one opaque enrollment template from multiple same-finger captures.
///
/// The output is a compact multi-view template bundle (`FPM1`) that
/// contains all provided ISO templates. `verify` understands this
/// format and compares a probe template against all bundled views.
pub fn enrollment_bundle(templates: &[Vec<u8>]) -> Result<Vec<u8>> {
    if templates.len() < 2 {
        return Err(FpError::ExtractFail(
            "need at least 2 templates for enrollment bundle".into(),
        ));
    }
    if templates.len() > TEMPLATE_BUNDLE_MAX_VIEWS {
        return Err(FpError::ExtractFail(format!(
            "too many templates: {} (max {})",
            templates.len(),
            TEMPLATE_BUNDLE_MAX_VIEWS
        )));
    }

    let mut out = Vec::new();
    out.extend_from_slice(TEMPLATE_BUNDLE_MAGIC);
    out.push(TEMPLATE_BUNDLE_VERSION);
    out.push(templates.len() as u8);

    for t in templates {
        if t.is_empty() {
            return Err(FpError::ExtractFail(
                "empty template in enrollment set".into(),
            ));
        }
        let len_u32 = u32::try_from(t.len())
            .map_err(|_| FpError::ExtractFail("template too large for enrollment bundle".into()))?;
        out.extend_from_slice(&len_u32.to_be_bytes());
        out.extend_from_slice(t);
    }

    Ok(out)
}

fn verify_iso_pair(extractor: &nbis::NbisExtractor, tmpl_a: &[u8], tmpl_b: &[u8]) -> Result<f64> {
    let m_a = extractor
        .load_iso_19794_2_2005(tmpl_a)
        .map_err(|e| FpError::ExtractFail(format!("failed to load template A: {}", e)))?;

    let m_b = extractor
        .load_iso_19794_2_2005(tmpl_b)
        .map_err(|e| FpError::ExtractFail(format!("failed to load template B: {}", e)))?;

    let raw_score = m_a.compare(&m_b);
    debug_log!("[verify] raw BOZORTH3 score: {}", raw_score);

    // Normalise: Bozorth3 returns an integer.  Scores above ~40 are
    // considered same-finger; scores can go into the hundreds.
    // We clamp to 400 and map to [0, 1].
    const MAX_SCORE: f64 = 400.0;
    Ok((raw_score as f64 / MAX_SCORE).clamp(0.0, 1.0))
}

fn new_extractor() -> Result<nbis::NbisExtractor> {
    use nbis::{NbisExtractor, NbisExtractorSettings};

    let settings = NbisExtractorSettings {
        min_quality: 0.0,
        get_center: false,
        check_fingerprint: false,
        compute_nfiq2: false,
        ppi: Some(SENSOR_DPI as f64),
    };

    NbisExtractor::new(settings)
        .map_err(|e| FpError::ExtractFail(format!("failed to create extractor: {}", e)))
}

fn verify_with_extractor(
    extractor: &nbis::NbisExtractor,
    tmpl_a: &[u8],
    tmpl_b: &[u8],
) -> Result<f64> {
    let views_a = template_views(tmpl_a)?;
    let views_b = template_views(tmpl_b)?;

    let mut scores = Vec::with_capacity(views_a.len().saturating_mul(views_b.len()));
    for a in &views_a {
        for b in &views_b {
            let score = verify_iso_pair(extractor, a, b)?;
            scores.push(score);
        }
    }

    if scores.is_empty() {
        return Err(FpError::ExtractFail("no templates to compare".into()));
    }

    let (best, second_best) = top_two_scores(&scores);
    let median = median_score(&scores);
    let final_score = aggregate_match_score(&scores);
    debug_log!(
        "[verify] compared {} pair(s), best={:.4}, second={:.4}, median={:.4}, final={:.4}",
        scores.len(),
        best,
        second_best,
        median,
        final_score
    );

    Ok(final_score)
}

fn aggregate_match_score(scores: &[f64]) -> f64 {
    match scores.len() {
        0 => 0.0,
        1 => scores[0],
        _ => median_score(scores),
    }
}

fn median_score(scores: &[f64]) -> f64 {
    if scores.is_empty() {
        return 0.0;
    }
    let mut sorted = scores.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

fn top_two_scores(scores: &[f64]) -> (f64, f64) {
    let mut best = 0.0_f64;
    let mut second_best = 0.0_f64;
    for &score in scores {
        if score >= best {
            second_best = best;
            best = score;
        } else if score > second_best {
            second_best = score;
        }
    }
    (best, second_best)
}

fn template_views(raw: &[u8]) -> Result<Vec<&[u8]>> {
    match decode_enrollment_bundle(raw)? {
        Some(views) => Ok(views),
        None => Ok(vec![raw]),
    }
}

fn decode_enrollment_bundle(raw: &[u8]) -> Result<Option<Vec<&[u8]>>> {
    if raw.len() < 6 || &raw[..4] != TEMPLATE_BUNDLE_MAGIC {
        return Ok(None);
    }

    let version = raw[4];
    if version != TEMPLATE_BUNDLE_VERSION {
        return Err(FpError::ExtractFail(format!(
            "unsupported enrollment bundle version: {}",
            version
        )));
    }

    let count = raw[5] as usize;
    if count == 0 {
        return Err(FpError::ExtractFail(
            "invalid enrollment bundle: zero views".into(),
        ));
    }
    if count > TEMPLATE_BUNDLE_MAX_VIEWS {
        return Err(FpError::ExtractFail(format!(
            "invalid enrollment bundle: too many views ({})",
            count
        )));
    }

    let mut views = Vec::with_capacity(count);
    let mut off = 6usize;
    for _ in 0..count {
        if off + 4 > raw.len() {
            return Err(FpError::ExtractFail(
                "invalid enrollment bundle: truncated length".into(),
            ));
        }
        let len = u32::from_be_bytes([raw[off], raw[off + 1], raw[off + 2], raw[off + 3]]) as usize;
        off += 4;
        if len == 0 || off + len > raw.len() {
            return Err(FpError::ExtractFail(
                "invalid enrollment bundle: invalid view payload".into(),
            ));
        }
        views.push(&raw[off..off + len]);
        off += len;
    }

    if off != raw.len() {
        return Err(FpError::ExtractFail(
            "invalid enrollment bundle: trailing bytes".into(),
        ));
    }

    Ok(Some(views))
}

#[derive(Clone, Copy, Debug)]
struct IsoMinutia {
    min_type: u8, // 1 = ridge ending, 2 = bifurcation
    x: u16,
    y: u16,
    angle: u8,
    quality: u8,
}

#[derive(Clone, Debug)]
struct IsoTemplate {
    width: u16,
    height: u16,
    finger_position: u8,
    view_and_impression: u8,
    finger_quality: u8,
    minutiae: Vec<IsoMinutia>,
}

#[derive(Clone, Debug)]
struct MinutiaCluster {
    min_type: u8,
    sum_x: f64,
    sum_y: f64,
    sum_quality: f64,
    sum_sin: f64,
    sum_cos: f64,
    count: usize,
    support_mask: u64,
    support_count: usize,
}

impl MinutiaCluster {
    fn new(m: IsoMinutia, scan_idx: usize) -> Self {
        let rad = angle_to_radians(m.angle);
        let mask = scan_bit(scan_idx);
        Self {
            min_type: m.min_type,
            sum_x: f64::from(m.x),
            sum_y: f64::from(m.y),
            sum_quality: f64::from(m.quality),
            sum_sin: rad.sin(),
            sum_cos: rad.cos(),
            count: 1,
            support_mask: mask,
            support_count: usize::from(mask != 0),
        }
    }

    fn add(&mut self, m: IsoMinutia, scan_idx: usize) {
        self.sum_x += f64::from(m.x);
        self.sum_y += f64::from(m.y);
        self.sum_quality += f64::from(m.quality);
        let rad = angle_to_radians(m.angle);
        self.sum_sin += rad.sin();
        self.sum_cos += rad.cos();
        self.count += 1;

        let bit = scan_bit(scan_idx);
        if bit != 0 && (self.support_mask & bit) == 0 {
            self.support_mask |= bit;
            self.support_count += 1;
        }
    }

    fn centroid(&self) -> (u16, u16, u8) {
        let x = clamp_u16((self.sum_x / self.count as f64).round());
        let y = clamp_u16((self.sum_y / self.count as f64).round());
        let angle = angle_from_vector(self.sum_sin, self.sum_cos);
        (x, y, angle)
    }

    fn avg_quality(&self) -> u8 {
        clamp_u8((self.sum_quality / self.count as f64).round(), 0, 63)
    }
}

/// Merge multiple same-finger templates into one fused enrollment template.
///
/// This aggregates minutiae across captures by clustering nearby
/// points (position + angle + type), prioritises clusters seen in
/// multiple scans, and emits a single ISO/IEC 19794-2:2005 template.
///
/// The resulting template is intended for enrollment storage.
pub fn merge_templates(templates: &[Vec<u8>]) -> Result<Vec<u8>> {
    if templates.len() < 2 {
        return Err(FpError::ExtractFail(
            "need at least 2 templates to merge".into(),
        ));
    }

    let mut parsed = Vec::with_capacity(templates.len());
    for raw in templates {
        parsed.push(parse_iso_template(raw)?);
    }
    let ref_idx = select_reference_template_index(templates)?;
    let base = parsed[ref_idx].clone();
    let base_center = template_center(&base);

    let mut clusters: Vec<MinutiaCluster> = Vec::new();
    for (scan_idx, tmpl) in parsed.iter().enumerate() {
        let (dx, dy) = match (base_center, template_center(tmpl)) {
            (Some((bx, by)), Some((tx, ty))) => (bx - tx, by - ty),
            _ => (0.0, 0.0),
        };

        for &m in &tmpl.minutiae {
            let m = translated_minutia(m, dx, dy, base.width, base.height);
            let mut attached = false;
            for c in &mut clusters {
                if c.min_type != m.min_type {
                    continue;
                }
                let (cx, cy, cang) = c.centroid();
                if (i32::from(m.x) - i32::from(cx)).abs() > MERGE_CLUSTER_TOLERANCE_PX {
                    continue;
                }
                if (i32::from(m.y) - i32::from(cy)).abs() > MERGE_CLUSTER_TOLERANCE_PX {
                    continue;
                }
                if angle_distance(m.angle, cang) > MERGE_CLUSTER_TOLERANCE_ANGLE {
                    continue;
                }

                c.add(m, scan_idx);
                attached = true;
                break;
            }

            if !attached {
                clusters.push(MinutiaCluster::new(m, scan_idx));
            }
        }
    }

    let mut ranked: Vec<(IsoMinutia, usize, u8)> = clusters
        .iter()
        .map(|c| {
            let (x, y, angle) = c.centroid();
            let quality = c.avg_quality();
            (
                IsoMinutia {
                    min_type: c.min_type,
                    x,
                    y,
                    angle,
                    quality,
                },
                c.support_count,
                quality,
            )
        })
        .collect();

    // Prefer minutiae seen in more scans, then by quality.
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));

    let min_support = min_support_for_scan_count(templates.len());
    let mut merged_minutiae: Vec<IsoMinutia> = ranked
        .iter()
        .filter(|(_, support, _)| *support >= min_support)
        .map(|(m, _, _)| *m)
        .collect();

    if merged_minutiae.len() < MIN_STABLE_MINUTIAE_TARGET {
        merged_minutiae = ranked.iter().map(|(m, _, _)| *m).collect();
    }

    merged_minutiae.truncate(MAX_ENROLLMENT_MINUTIAE);

    debug_log!(
        "[merge] ref_idx={}, scans={}, clusters={}, selected={}, min_support={}",
        ref_idx,
        templates.len(),
        clusters.len(),
        merged_minutiae.len(),
        min_support
    );

    if merged_minutiae.len() > MAX_BOZORTH_MINUTIAE {
        merged_minutiae.truncate(MAX_BOZORTH_MINUTIAE);
    }

    if merged_minutiae.is_empty() {
        return Err(FpError::ExtractFail(
            "merge produced empty template".to_string(),
        ));
    }

    let merged = IsoTemplate {
        width: base.width,
        height: base.height,
        finger_position: base.finger_position,
        view_and_impression: base.view_and_impression,
        finger_quality: avg_finger_quality(&parsed),
        minutiae: merged_minutiae,
    };

    Ok(encode_iso_template(&merged))
}

fn select_reference_template_index(templates: &[Vec<u8>]) -> Result<usize> {
    if templates.is_empty() {
        return Err(FpError::ExtractFail("no templates supplied".into()));
    }
    if templates.len() == 1 {
        return Ok(0);
    }

    let mut best_idx = 0usize;
    let mut best_avg = f64::MIN;

    for i in 0..templates.len() {
        let mut sum = 0.0_f64;
        let mut count = 0usize;
        for j in 0..templates.len() {
            if i == j {
                continue;
            }
            sum += verify(&templates[i], &templates[j])?;
            count += 1;
        }

        let avg = if count == 0 { 0.0 } else { sum / count as f64 };
        if avg > best_avg {
            best_avg = avg;
            best_idx = i;
        }
    }

    Ok(best_idx)
}

fn template_center(t: &IsoTemplate) -> Option<(f64, f64)> {
    if t.minutiae.is_empty() {
        return None;
    }
    let (sx, sy) = t.minutiae.iter().fold((0.0_f64, 0.0_f64), |acc, m| {
        (acc.0 + f64::from(m.x), acc.1 + f64::from(m.y))
    });
    let n = t.minutiae.len() as f64;
    Some((sx / n, sy / n))
}

fn translated_minutia(m: IsoMinutia, dx: f64, dy: f64, width: u16, height: u16) -> IsoMinutia {
    let max_x = width.saturating_sub(1) as f64;
    let max_y = height.saturating_sub(1) as f64;
    let x = (f64::from(m.x) + dx).round().clamp(0.0, max_x) as u16;
    let y = (f64::from(m.y) + dy).round().clamp(0.0, max_y) as u16;
    IsoMinutia { x, y, ..m }
}

fn min_support_for_scan_count(scan_count: usize) -> usize {
    if scan_count >= 8 {
        3
    } else if scan_count >= 4 {
        2
    } else {
        1
    }
}

fn parse_iso_template(raw: &[u8]) -> Result<IsoTemplate> {
    if raw.len() < ISO_TEMPLATE_MIN_LEN {
        return Err(FpError::ExtractFail("ISO template too short".into()));
    }
    if &raw[..8] != ISO_TEMPLATE_MAGIC {
        return Err(FpError::ExtractFail("invalid ISO template header".into()));
    }

    let total_len = u32::from_be_bytes([raw[8], raw[9], raw[10], raw[11]]) as usize;
    if total_len != raw.len() {
        return Err(FpError::ExtractFail("ISO template length mismatch".into()));
    }

    let width = u16::from_be_bytes([raw[14], raw[15]]);
    let height = u16::from_be_bytes([raw[16], raw[17]]);
    let finger_position = raw[18];
    let view_and_impression = raw[19];
    let finger_quality = raw[20];
    let num_minutiae = raw[25] as usize;

    let expected_len = ISO_TEMPLATE_HEADER_LEN + num_minutiae * 6;
    if expected_len != raw.len() {
        return Err(FpError::ExtractFail(
            "ISO template minutiae length mismatch".into(),
        ));
    }

    let mut minutiae = Vec::with_capacity(num_minutiae);
    for i in 0..num_minutiae {
        let off = ISO_TEMPLATE_HEADER_LEN + i * 6;
        let b0 = raw[off];
        let min_type = match (b0 & 0xC0) >> 6 {
            0 => 1, // Treat unknown as ridge ending.
            t => t,
        };
        let x = (u16::from(b0 & 0x3F) << 8) | u16::from(raw[off + 1]);
        let y = (u16::from(raw[off + 2]) << 8) | u16::from(raw[off + 3]);
        let angle = raw[off + 4];
        let quality = raw[off + 5];
        minutiae.push(IsoMinutia {
            min_type,
            x,
            y,
            angle,
            quality,
        });
    }

    Ok(IsoTemplate {
        width,
        height,
        finger_position,
        view_and_impression,
        finger_quality,
        minutiae,
    })
}

fn encode_iso_template(t: &IsoTemplate) -> Vec<u8> {
    let count = t.minutiae.len().min(MAX_BOZORTH_MINUTIAE);
    let total_len = ISO_TEMPLATE_HEADER_LEN + count * 6;

    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(ISO_TEMPLATE_MAGIC);
    out.extend_from_slice(&(total_len as u32).to_be_bytes());
    out.extend_from_slice(&[0x00, 0x00]); // reserved
    out.extend_from_slice(&t.width.to_be_bytes());
    out.extend_from_slice(&t.height.to_be_bytes());
    out.push(t.finger_position);
    out.push(t.view_and_impression);
    out.push(t.finger_quality);
    out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // reserved
    out.push(count as u8);

    for m in t.minutiae.iter().take(count) {
        let x = m.x & 0x3FFF; // 14-bit storage
        let type_bits = (m.min_type & 0x03) << 6;
        out.push(type_bits | ((x >> 8) as u8 & 0x3F));
        out.push((x & 0xFF) as u8);
        out.push((m.y >> 8) as u8);
        out.push((m.y & 0xFF) as u8);
        out.push(m.angle);
        out.push(m.quality.min(63));
    }

    out
}

fn avg_finger_quality(templates: &[IsoTemplate]) -> u8 {
    let sum: u32 = templates.iter().map(|t| u32::from(t.finger_quality)).sum();
    (sum / templates.len() as u32) as u8
}

fn scan_bit(scan_idx: usize) -> u64 {
    if scan_idx >= 64 {
        0
    } else {
        1u64 << scan_idx
    }
}

fn clamp_u8(value: f64, min: u8, max: u8) -> u8 {
    let v = value as i32;
    if v < i32::from(min) {
        min
    } else if v > i32::from(max) {
        max
    } else {
        v as u8
    }
}

fn clamp_u16(value: f64) -> u16 {
    let v = value as i64;
    if v < 0 {
        0
    } else if v > i64::from(u16::MAX) {
        u16::MAX
    } else {
        v as u16
    }
}

fn angle_to_radians(code: u8) -> f64 {
    f64::from(code) * (2.0 * PI / 256.0)
}

fn angle_from_vector(sum_sin: f64, sum_cos: f64) -> u8 {
    if sum_sin == 0.0 && sum_cos == 0.0 {
        return 0;
    }
    let mut theta = sum_sin.atan2(sum_cos);
    if theta < 0.0 {
        theta += 2.0 * PI;
    }
    let code = ((theta * 256.0 / (2.0 * PI)).round() as i32) & 0xFF;
    code as u8
}

fn angle_distance(a: u8, b: u8) -> i32 {
    let mut diff = (i32::from(a) - i32::from(b)).abs();
    if diff > 128 {
        diff = 256 - diff;
    }
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_template(points: &[(u8, u16, u16, u8, u8)]) -> Vec<u8> {
        let minutiae = points
            .iter()
            .map(|(min_type, x, y, angle, quality)| IsoMinutia {
                min_type: *min_type,
                x: *x,
                y: *y,
                angle: *angle,
                quality: *quality,
            })
            .collect::<Vec<_>>();

        encode_iso_template(&IsoTemplate {
            width: IMAGE_WIDTH as u16,
            height: IMAGE_HEIGHT as u16,
            finger_position: 0,
            view_and_impression: 0,
            finger_quality: 60,
            minutiae,
        })
    }

    #[test]
    fn merge_templates_requires_two_inputs() {
        let one = vec![make_template(&[(1, 100, 100, 10, 50)])];
        let err = merge_templates(&one).expect_err("expected merge to fail");
        assert!(matches!(err, FpError::ExtractFail(_)));
    }

    #[test]
    fn merge_templates_combines_coverage_from_multiple_scans() {
        // One overlapping point near (100,100) and two unique regions.
        let t1 = make_template(&[(1, 100, 100, 20, 55), (1, 40, 200, 10, 50)]);
        let t2 = make_template(&[(1, 102, 98, 22, 57), (2, 300, 80, 100, 45)]);

        let merged = merge_templates(&[t1, t2]).expect("merge should succeed");
        let parsed = parse_iso_template(&merged).expect("merged template should be valid");

        // Overlap should collapse into one cluster, plus two unique points.
        assert!(
            parsed.minutiae.len() >= 3,
            "expected merged template to retain combined coverage, got {} minutiae",
            parsed.minutiae.len()
        );

        let _self_score = verify(&merged, &merged).expect("self verify should work");
    }

    #[test]
    fn merge_templates_aligns_center_shifted_scans() {
        // Template 2 is translated by roughly (+20, +15) vs template 1.
        let t1 = make_template(&[(1, 100, 100, 20, 50), (2, 130, 110, 80, 45)]);
        let t2 = make_template(&[(1, 120, 115, 22, 52), (2, 150, 125, 82, 47)]);

        let merged = merge_templates(&[t1, t2]).expect("merge should succeed");
        let parsed = parse_iso_template(&merged).expect("merged template should parse");

        // After center alignment + clustering, these should collapse to ~2 stable points.
        assert!(
            parsed.minutiae.len() <= 3,
            "expected strong cluster merge, got {} minutiae",
            parsed.minutiae.len()
        );
    }

    #[test]
    fn enrollment_bundle_round_trip_views() {
        let t1 = make_template(&[(1, 100, 100, 20, 50)]);
        let t2 = make_template(&[(2, 120, 90, 80, 40)]);

        let bundle = enrollment_bundle(&[t1.clone(), t2.clone()]).expect("bundle should build");
        let views = decode_enrollment_bundle(&bundle)
            .expect("bundle should parse")
            .expect("bundle marker should be detected");
        assert_eq!(views.len(), 2);
        assert_eq!(views[0], t1.as_slice());
        assert_eq!(views[1], t2.as_slice());
    }

    #[test]
    fn verify_bundle_uses_median_consensus() {
        let t1 = make_template(&[(1, 100, 100, 20, 50), (2, 130, 110, 80, 45)]);
        let t2 = make_template(&[(1, 220, 150, 40, 50), (2, 240, 170, 90, 40)]);
        let probe = t1.clone();

        let s1 = verify(&t1, &probe).expect("verify t1/probe");
        let s2 = verify(&t2, &probe).expect("verify t2/probe");
        let expected = (s1.max(s2) + s1.min(s2)) / 2.0;

        let bundle = enrollment_bundle(&[t1, t2]).expect("bundle should build");
        let got = verify(&bundle, &probe).expect("verify bundle/probe");

        assert!(
            (got - expected).abs() < 1e-9,
            "bundle score {} should equal median consensus score {}",
            got,
            expected
        );
    }

    #[test]
    fn aggregate_match_score_reduces_single_view_spike() {
        let scores = [0.065, 0.020, 0.000];
        let got = aggregate_match_score(&scores);
        assert!(
            got < 0.06,
            "median consensus score should drop below 0.06 for one-view spike, got {}",
            got
        );
    }

    #[test]
    fn aggregate_match_score_uses_median_for_even_count() {
        let scores = [0.1250, 0.0625, 0.0525, 0.0500, 0.0400, 0.0300];
        let got = aggregate_match_score(&scores);
        let expected = (0.0500 + 0.0525) / 2.0;
        assert!(
            (got - expected).abs() < 1e-9,
            "expected median score {}, got {}",
            expected,
            got
        );
    }

    #[test]
    fn identify_best_returns_matching_candidate_index() {
        let probe = make_template(&[(1, 100, 100, 20, 50), (2, 130, 110, 80, 45)]);
        let c1 = make_template(&[(1, 220, 150, 40, 50), (2, 240, 170, 90, 40)]);
        let c2 = probe.clone();
        let candidates = vec![c1.as_slice(), c2.as_slice()];

        let (idx, score) = identify_best(&probe, &candidates).expect("identify should succeed");
        let s0 = verify(&probe, candidates[0]).expect("score candidate 0");
        let s1 = verify(&probe, candidates[1]).expect("score candidate 1");
        let expected_idx = if s1 > s0 { 1 } else { 0 };
        let expected_score = s0.max(s1);

        assert_eq!(idx, expected_idx, "best index should match highest score");
        assert!(
            (score - expected_score).abs() < 1e-9,
            "best score should equal max candidate score"
        );
    }

    #[test]
    fn identify_best_errors_on_empty_candidate_list() {
        let probe = make_template(&[(1, 100, 100, 20, 50)]);
        let err = identify_best(&probe, &[]).expect_err("empty candidates should fail");
        assert!(matches!(err, FpError::ExtractFail(_)));
    }
}
