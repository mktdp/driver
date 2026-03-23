//! Hardware test: 6-scan enrollment package + 1 verification scan.
//!
//! Flow:
//! 1. Capture 6 templates from the SAME finger
//! 2. Combine the 6 templates into one enrollment package
//! 3. Capture one more template and compare against that package
//!
//! Run with:
//!   cargo run --example hw_enroll_merge_verify --features hardware-tests

use std::thread;
use std::time::Duration;

use mktdp_driver::{biometric, driver, image};

const DEFAULT_TIMEOUT_MS: u32 = 10_000;
const ENROLL_SCANS: usize = 6;
const DEFAULT_THRESHOLD: f64 = 0.06;
const DEFAULT_VERIFY_TRIES: usize = 3;

fn main() {
    println!("=== MKTDP Driver — 6-Scan Enrollment Package Test ===\n");
    let threshold = match_threshold_from_env();
    let verify_tries = verify_tries_from_env();
    println!(
        "Match threshold: {:.4} (set FP_MATCH_THRESHOLD to override)\n",
        threshold
    );
    println!(
        "Probe retries: {} (set FP_VERIFY_TRIES to override)\n",
        verify_tries
    );

    println!("[1/5] Opening scanner...");
    let mut dev = match driver::open() {
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
        "[2/5] Capture SAME finger {} times for enrollment:",
        ENROLL_SCANS
    );
    let mut enrollment_templates = Vec::with_capacity(ENROLL_SCANS);
    for idx in 0..ENROLL_SCANS {
        println!(
            "  [enroll {}/{}] Place the SAME finger now (timeout: {} ms)",
            idx + 1,
            ENROLL_SCANS,
            DEFAULT_TIMEOUT_MS
        );

        match capture_template(&mut dev, DEFAULT_TIMEOUT_MS) {
            Ok(t) => {
                println!("    -> template: {} bytes", t.len());
                enrollment_templates.push(t);
            }
            Err(e) => {
                eprintln!("    -> capture failed: {}", e);
                driver::close(dev);
                std::process::exit(1);
            }
        }

        if idx + 1 < ENROLL_SCANS {
            thread::sleep(Duration::from_millis(1200));
        }
    }
    println!();

    println!("[3/5] Building enrollment template package...");
    let enrollment_package = match biometric::enrollment_bundle(&enrollment_templates) {
        Ok(t) => {
            println!("  ✓ Enrollment template package: {} bytes\n", t.len());
            t
        }
        Err(e) => {
            eprintln!("  ✗ Enrollment package build failed: {}", e);
            driver::close(dev);
            std::process::exit(1);
        }
    };

    println!("[4/5] Capture one more scan and verify against enrollment package:");
    let mut best_score = 0.0_f64;
    let mut matched = false;
    for attempt in 1..=verify_tries {
        println!(
            "  [probe {}/{}] Place the SAME finger again (timeout: {} ms)",
            attempt, verify_tries, DEFAULT_TIMEOUT_MS
        );
        let probe = match capture_template(&mut dev, DEFAULT_TIMEOUT_MS) {
            Ok(t) => {
                println!("    -> probe template: {} bytes", t.len());
                t
            }
            Err(e) => {
                eprintln!("    -> probe capture failed: {}", e);
                driver::close(dev);
                std::process::exit(1);
            }
        };

        let score = match biometric::verify(&enrollment_package, &probe) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("  ✗ Verify failed: {}", e);
                driver::close(dev);
                std::process::exit(1);
            }
        };

        if score > best_score {
            best_score = score;
        }
        println!("    -> similarity score: {:.4}", score);

        if score >= threshold {
            println!("  ✓ MATCH (score >= {:.4})", threshold);
            matched = true;
            break;
        }

        if attempt < verify_tries {
            println!(
                "    -> below threshold ({:.4}); please scan again...",
                threshold
            );
            thread::sleep(Duration::from_millis(1000));
        }
    }

    if !matched {
        println!(
            "  → NO MATCH after {} probe attempts (best score: {:.4}, threshold: {:.4})",
            verify_tries, best_score, threshold
        );
    }

    println!("\n[5/5] Closing scanner...");
    driver::close(dev);
    println!("  ✓ Done.");
    println!("\n=== Enrollment package test complete ===");
}

fn capture_template(dev: &mut driver::FpDevice, timeout_ms: u32) -> Result<Vec<u8>, String> {
    let raw_frame = driver::scan(dev, timeout_ms).map_err(|e| format!("scan: {}", e))?;
    let grayscale = image::deframe(&raw_frame).map_err(|e| format!("deframe: {}", e))?;
    biometric::extract(&grayscale).map_err(|e| format!("extract: {}", e))
}

fn match_threshold_from_env() -> f64 {
    match std::env::var("FP_MATCH_THRESHOLD") {
        Ok(v) => v
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|t| (0.0..=1.0).contains(t))
            .unwrap_or(DEFAULT_THRESHOLD),
        Err(_) => DEFAULT_THRESHOLD,
    }
}

fn verify_tries_from_env() -> usize {
    match std::env::var("FP_VERIFY_TRIES") {
        Ok(v) => v
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|n| *n >= 1 && *n <= 10)
            .unwrap_or(DEFAULT_VERIFY_TRIES),
        Err(_) => DEFAULT_VERIFY_TRIES,
    }
}
