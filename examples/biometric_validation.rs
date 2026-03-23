//! Interactive biometric score validation harness.
//!
//! This tool captures multiple templates for:
//! - Finger A (same finger, repeated scans)
//! - Finger B (a different finger, repeated scans)
//!
//! It then computes:
//! - Genuine scores: all pairwise comparisons within A (and within B)
//! - Impostor scores: all cross comparisons A × B
//!
//! Run:
//!   cargo run --example biometric_validation --features hardware-tests
//!
//! Optional args:
//!   --same <N>      Number of captures for finger A (default: 10)
//!   --diff <N>      Number of captures for finger B (default: 10)
//!   --timeout <MS>  Per-capture finger timeout (default: 10000)

use std::cmp::Ordering;
use std::env;
use std::thread;
use std::time::Duration;

use fingerprint_driver::{biometric, image, usb};

const DEFAULT_SAME_COUNT: usize = 10;
const DEFAULT_DIFF_COUNT: usize = 10;
const DEFAULT_TIMEOUT_MS: u32 = 10_000;
const DEFAULT_THRESHOLD: f64 = 0.06;
const RAW_SCORE_SCALE: f64 = 400.0;
const MAX_CAPTURE_RETRIES_PER_SAMPLE: usize = 4;
const DEFAULT_FAR_TARGET: f64 = 0.01; // 1%
const ENV_MATCH_THRESHOLD: &str = "FP_MATCH_THRESHOLD";

struct Config {
    same_count: usize,
    diff_count: usize,
    timeout_ms: u32,
}

#[derive(Clone, Copy)]
struct ScoreStats {
    count: usize,
    min: f64,
    max: f64,
    mean: f64,
    p50: f64,
    p90: f64,
}

fn main() {
    let cfg = match parse_args() {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("error: {}", msg);
            std::process::exit(2);
        }
    };

    println!("=== Fingerprint Driver — Biometric Validation ===\n");
    let threshold = match_threshold_from_env();
    println!("Config:");
    println!("  same-finger captures (A): {}", cfg.same_count);
    println!("  different-finger captures (B): {}", cfg.diff_count);
    println!("  timeout per capture: {} ms\n", cfg.timeout_ms);
    println!(
        "  match threshold: {:.4} (set {} to override)\n",
        threshold, ENV_MATCH_THRESHOLD
    );

    println!("[1/5] Opening scanner...");
    let mut dev = match usb::open() {
        Ok(d) => {
            println!("  ✓ Scanner opened.\n");
            d
        }
        Err(e) => {
            eprintln!("  ✗ Failed to open scanner: {}", e);
            std::process::exit(1);
        }
    };

    println!(
        "[2/5] Capture finger A {}× (use the SAME finger each time).",
        cfg.same_count
    );
    let templates_a = match capture_set(&mut dev, "A", cfg.same_count, cfg.timeout_ms) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  ✗ Capture set A failed: {}", e);
            usb::close(dev);
            std::process::exit(1);
        }
    };
    println!();

    let templates_b = if cfg.diff_count > 0 {
        println!(
            "[3/5] Capture finger B {}× (use a DIFFERENT finger from A).",
            cfg.diff_count
        );
        println!("  Lift finger A now, then place finger B.");
        thread::sleep(Duration::from_secs(3));
        match capture_set(&mut dev, "B", cfg.diff_count, cfg.timeout_ms) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("  ✗ Capture set B failed: {}", e);
                usb::close(dev);
                std::process::exit(1);
            }
        }
    } else {
        Vec::new()
    };
    println!();

    println!("[4/5] Computing score distributions...");
    let same_a_scores = match pairwise_scores(&templates_a) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("  ✗ Failed to score set A: {}", e);
            usb::close(dev);
            std::process::exit(1);
        }
    };

    let same_b_scores = match pairwise_scores(&templates_b) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("  ✗ Failed to score set B: {}", e);
            usb::close(dev);
            std::process::exit(1);
        }
    };

    let cross_scores = match cross_scores(&templates_a, &templates_b) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("  ✗ Failed cross scoring A×B: {}", e);
            usb::close(dev);
            std::process::exit(1);
        }
    };

    let mut genuine_scores = same_a_scores.clone();
    genuine_scores.extend_from_slice(&same_b_scores);

    print_stats("Same finger A", &same_a_scores);
    if !same_b_scores.is_empty() {
        print_stats("Same finger B", &same_b_scores);
    }
    if !cross_scores.is_empty() {
        print_stats("Different fingers A×B", &cross_scores);
    }

    println!("\nThreshold check:");
    print_threshold_metrics(threshold, "default", &genuine_scores, &cross_scores);

    println!("\nThreshold sweep:");
    for t in [0.02_f64, 0.04, 0.06, 0.08, 0.10, 0.12] {
        print_threshold_metrics(t, "candidate", &genuine_scores, &cross_scores);
    }

    if let Some(rec) = recommend_threshold(&genuine_scores, &cross_scores, DEFAULT_FAR_TARGET) {
        print_threshold_metrics(rec, "recommended", &genuine_scores, &cross_scores);
        println!(
            "  recommendation policy: lowest FRR with FAR <= {:.2}%",
            DEFAULT_FAR_TARGET * 100.0
        );
    }

    println!("\n[5/5] Closing scanner...");
    usb::close(dev);
    println!("  ✓ Done.");
    println!("\n=== Validation complete ===");
}

fn parse_args() -> Result<Config, String> {
    let mut cfg = Config {
        same_count: DEFAULT_SAME_COUNT,
        diff_count: DEFAULT_DIFF_COUNT,
        timeout_ms: DEFAULT_TIMEOUT_MS,
    };

    let args: Vec<String> = env::args().collect();
    let mut i = 1usize;
    while i < args.len() {
        match args[i].as_str() {
            "--same" => {
                i += 1;
                let v = args.get(i).ok_or("--same requires a value".to_string())?;
                cfg.same_count = v
                    .parse::<usize>()
                    .map_err(|_| "--same must be a positive integer".to_string())?;
            }
            "--diff" => {
                i += 1;
                let v = args.get(i).ok_or("--diff requires a value".to_string())?;
                cfg.diff_count = v
                    .parse::<usize>()
                    .map_err(|_| "--diff must be a non-negative integer".to_string())?;
            }
            "--timeout" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or("--timeout requires a value".to_string())?;
                cfg.timeout_ms = v
                    .parse::<u32>()
                    .map_err(|_| "--timeout must be a non-negative integer".to_string())?;
            }
            "--help" | "-h" => {
                return Err(
                    "usage: biometric_validation [--same N] [--diff N] [--timeout MS]".into(),
                );
            }
            other => {
                return Err(format!("unknown argument: {}", other));
            }
        }
        i += 1;
    }

    if cfg.same_count < 2 {
        return Err("--same must be >= 2 for pairwise scoring".into());
    }

    Ok(cfg)
}

fn match_threshold_from_env() -> f64 {
    match std::env::var(ENV_MATCH_THRESHOLD) {
        Ok(v) => v
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|t| (0.0..=1.0).contains(t))
            .unwrap_or(DEFAULT_THRESHOLD),
        Err(_) => DEFAULT_THRESHOLD,
    }
}

fn capture_set(
    dev: &mut usb::FpDevice,
    label: &str,
    count: usize,
    timeout_ms: u32,
) -> Result<Vec<Vec<u8>>, String> {
    let mut templates = Vec::with_capacity(count);

    for idx in 0..count {
        let mut captured = None;
        for attempt in 1..=MAX_CAPTURE_RETRIES_PER_SAMPLE {
            println!(
                "  [{} {}/{}] Place finger now (timeout: {} ms) [attempt {}/{}]",
                label,
                idx + 1,
                count,
                timeout_ms,
                attempt,
                MAX_CAPTURE_RETRIES_PER_SAMPLE
            );
            match capture_template(dev, timeout_ms) {
                Ok(tmpl) => {
                    // Reject malformed templates up-front so scoring phase
                    // doesn't fail halfway through a long capture session.
                    if let Err(e) = biometric::verify(&tmpl, &tmpl) {
                        println!("    -> unusable template (self-verify failed: {})", e);
                        thread::sleep(Duration::from_millis(900));
                        continue;
                    }
                    println!("    -> template: {} bytes", tmpl.len());
                    captured = Some(tmpl);
                    break;
                }
                Err(e) => {
                    println!("    -> capture failed: {}", e);
                    thread::sleep(Duration::from_millis(900));
                }
            }
        }

        let tmpl = captured.ok_or_else(|| {
            format!(
                "failed to capture a valid template for {} sample {} after {} attempts",
                label,
                idx + 1,
                MAX_CAPTURE_RETRIES_PER_SAMPLE
            )
        })?;

        templates.push(tmpl);
        thread::sleep(Duration::from_millis(1200));
    }

    Ok(templates)
}

fn capture_template(dev: &mut usb::FpDevice, timeout_ms: u32) -> Result<Vec<u8>, String> {
    let raw_frame = usb::scan(dev, timeout_ms).map_err(|e| format!("scan: {}", e))?;
    let grayscale = image::deframe(&raw_frame).map_err(|e| format!("deframe: {}", e))?;
    biometric::extract(&grayscale).map_err(|e| format!("extract: {}", e))
}

fn pairwise_scores(templates: &[Vec<u8>]) -> Result<Vec<f64>, String> {
    if templates.len() < 2 {
        return Ok(Vec::new());
    }

    let mut scores = Vec::new();
    for i in 0..templates.len() {
        for j in (i + 1)..templates.len() {
            let score = biometric::verify(&templates[i], &templates[j])
                .map_err(|e| format!("verify({},{}) failed: {}", i, j, e))?;
            scores.push(score);
        }
    }
    Ok(scores)
}

fn cross_scores(a: &[Vec<u8>], b: &[Vec<u8>]) -> Result<Vec<f64>, String> {
    if a.is_empty() || b.is_empty() {
        return Ok(Vec::new());
    }

    let mut scores = Vec::new();
    for (i, ta) in a.iter().enumerate() {
        for (j, tb) in b.iter().enumerate() {
            let score = biometric::verify(ta, tb)
                .map_err(|e| format!("verify(A{},B{}) failed: {}", i, j, e))?;
            scores.push(score);
        }
    }
    Ok(scores)
}

fn summarize(scores: &[f64]) -> Option<ScoreStats> {
    if scores.is_empty() {
        return None;
    }
    let mut sorted = scores.to_vec();
    sorted.sort_by(|a, b| {
        if a.is_nan() && b.is_nan() {
            Ordering::Equal
        } else if a.is_nan() {
            Ordering::Greater
        } else if b.is_nan() {
            Ordering::Less
        } else {
            a.total_cmp(b)
        }
    });

    let count = sorted.len();
    let min = sorted[0];
    let max = sorted[count - 1];
    let sum: f64 = sorted.iter().sum();
    let mean = sum / count as f64;
    let p50 = percentile(&sorted, 0.50);
    let p90 = percentile(&sorted, 0.90);

    Some(ScoreStats {
        count,
        min,
        max,
        mean,
        p50,
        p90,
    })
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let last = sorted.len() - 1;
    let idx = ((last as f64) * p).round() as usize;
    sorted[idx.min(last)]
}

fn print_stats(label: &str, scores: &[f64]) {
    println!("\n  {}:", label);
    match summarize(scores) {
        Some(s) => {
            println!("    pairs: {}", s.count);
            println!(
                "    min/mean/max: {:.4} / {:.4} / {:.4}",
                s.min, s.mean, s.max
            );
            println!("    p50/p90: {:.4} / {:.4}", s.p50, s.p90);
            println!(
                "    raw min/mean/max (~×400): {:.1} / {:.1} / {:.1}",
                s.min * RAW_SCORE_SCALE,
                s.mean * RAW_SCORE_SCALE,
                s.max * RAW_SCORE_SCALE
            );
        }
        None => println!("    (no scores)"),
    }
}

fn print_threshold_metrics(
    threshold: f64,
    label: &str,
    genuine_scores: &[f64],
    impostor_scores: &[f64],
) {
    let false_rejects = genuine_scores.iter().filter(|&&s| s < threshold).count();
    let false_accepts = impostor_scores.iter().filter(|&&s| s >= threshold).count();

    let frr = if genuine_scores.is_empty() {
        0.0
    } else {
        false_rejects as f64 / genuine_scores.len() as f64
    };
    let far = if impostor_scores.is_empty() {
        0.0
    } else {
        false_accepts as f64 / impostor_scores.len() as f64
    };

    println!(
        "  {} threshold {:.4} (~raw {:.1}): FRR {:.2}% ({}/{}), FAR {:.2}% ({}/{})",
        label,
        threshold,
        threshold * RAW_SCORE_SCALE,
        frr * 100.0,
        false_rejects,
        genuine_scores.len(),
        far * 100.0,
        false_accepts,
        impostor_scores.len()
    );
}

fn recommend_threshold(
    genuine_scores: &[f64],
    impostor_scores: &[f64],
    far_target: f64,
) -> Option<f64> {
    if genuine_scores.is_empty() || impostor_scores.is_empty() {
        return None;
    }

    // BOZORTH3 raw score is integer and we normalize by /400, so
    // candidate thresholds naturally lie on this grid.
    let mut candidates = Vec::new();
    for raw in 0..=400 {
        candidates.push(raw as f64 / RAW_SCORE_SCALE);
    }

    let mut best_with_far_target: Option<(f64, f64, f64)> = None; // (thr, frr, far)
    let mut best_fallback: Option<(f64, f64, f64)> = None; // min FAR, then min FRR

    for &thr in &candidates {
        let (frr, far) = rates_at_threshold(thr, genuine_scores, impostor_scores);

        if far <= far_target {
            match best_with_far_target {
                None => best_with_far_target = Some((thr, frr, far)),
                Some((best_thr, best_frr, _)) => {
                    if frr < best_frr || (frr == best_frr && thr < best_thr) {
                        best_with_far_target = Some((thr, frr, far));
                    }
                }
            }
        }

        match best_fallback {
            None => best_fallback = Some((thr, frr, far)),
            Some((best_thr, best_frr, best_far)) => {
                if far < best_far
                    || (far == best_far && frr < best_frr)
                    || (far == best_far && frr == best_frr && thr < best_thr)
                {
                    best_fallback = Some((thr, frr, far));
                }
            }
        }
    }

    best_with_far_target
        .or(best_fallback)
        .map(|(thr, _, _)| thr)
}

fn rates_at_threshold(
    threshold: f64,
    genuine_scores: &[f64],
    impostor_scores: &[f64],
) -> (f64, f64) {
    let false_rejects = genuine_scores.iter().filter(|&&s| s < threshold).count();
    let false_accepts = impostor_scores.iter().filter(|&&s| s >= threshold).count();

    let frr = if genuine_scores.is_empty() {
        0.0
    } else {
        false_rejects as f64 / genuine_scores.len() as f64
    };
    let far = if impostor_scores.is_empty() {
        0.0
    } else {
        false_accepts as f64 / impostor_scores.len() as f64
    };

    (frr, far)
}
