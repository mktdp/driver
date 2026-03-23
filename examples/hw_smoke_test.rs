//! Manual hardware smoke test.
//!
//! This binary talks to a real U.are.U 4500 scanner. Run it with:
//!
//!   cargo run --example hw_smoke_test
//!
//! It will:
//!   1. Open the scanner
//!   2. Wait for you to place your finger (10s timeout)
//!   3. Capture + extract a template
//!   4. Ask you to place the SAME finger again
//!   5. Capture + extract a second template
//!   6. Compare both templates and print the score
//!   7. Clean up

use std::path::Path;

use fingerprint_driver::{biometric, image, usb};

const DEFAULT_MATCH_THRESHOLD: f64 = 0.06;

fn main() {
    println!("=== Fingerprint Driver — Hardware Smoke Test ===\n");
    let threshold = match_threshold_from_env();
    println!(
        "Match threshold: {:.4} (set FP_MATCH_THRESHOLD to override)\n",
        threshold
    );

    // ── Step 1: Open device ────────────────────────────────────────
    println!("[1/6] Opening scanner...");
    let mut dev = match usb::open() {
        Ok(d) => {
            println!("  ✓ Scanner opened successfully.\n");
            d
        }
        Err(e) => {
            eprintln!("  ✗ Failed to open scanner: {}", e);
            eprintln!();
            eprintln!("Troubleshooting:");
            eprintln!("  • Is the U.are.U 4500 plugged in?  Run: lsusb | grep 05ba");
            eprintln!("  • Are udev rules installed?         Run: ls /etc/udev/rules.d/70-fingerprint.rules");
            eprintln!("  • Is your user in plugdev group?    Run: groups");
            eprintln!("  • Try running with sudo (just to test permissions).");
            std::process::exit(1);
        }
    };

    // ── Step 2: First scan ─────────────────────────────────────────
    println!("[2/6] Place your finger on the scanner... (10 second timeout)");
    let tmpl_a = match scan_and_extract(&mut dev, "A") {
        Ok(t) => t,
        Err(e) => {
            eprintln!("  ✗ First scan failed: {}", e);
            usb::close(dev);
            std::process::exit(1);
        }
    };
    println!("  ✓ Template A: {} bytes\n", tmpl_a.len());

    // ── Step 3: Second scan ────────────────────────────────────────
    println!("[3/6] Lift your finger, then place the SAME finger again... (10 second timeout)");
    // Small delay so the user has time to lift their finger.
    std::thread::sleep(std::time::Duration::from_secs(2));

    let tmpl_b = match scan_and_extract(&mut dev, "B") {
        Ok(t) => t,
        Err(e) => {
            eprintln!("  ✗ Second scan failed: {}", e);
            usb::close(dev);
            std::process::exit(1);
        }
    };
    println!("  ✓ Template B: {} bytes\n", tmpl_b.len());

    // ── Step 4: Verify ─────────────────────────────────────────────
    println!("[4/6] Comparing templates...");
    match biometric::verify(&tmpl_a, &tmpl_b) {
        Ok(score) => {
            println!("  ✓ Similarity score: {:.4} (range 0.0–1.0)\n", score);
            if score >= threshold {
                println!("  → MATCH (score >= {:.4} threshold)", threshold);
            } else {
                println!("  → NO MATCH (score < {:.4} threshold)", threshold);
                println!("    This might mean the two scans were different fingers,");
                println!("    or the finger placement was too different between scans.");
            }
        }
        Err(e) => {
            eprintln!("  ✗ Verification failed: {}", e);
        }
    }

    // ── Step 5: Close ──────────────────────────────────────────────
    println!("\n[5/6] Closing scanner...");
    usb::close(dev);
    println!("  ✓ Done.\n");

    println!("[6/6] Summary:");
    println!("  Template A size: {} bytes", tmpl_a.len());
    println!("  Template B size: {} bytes", tmpl_b.len());
    println!("\n=== Smoke test complete ===");
}

/// Capture a raw frame, deframe it, and extract a biometric template.
/// `label` is used to name the debug PNG saved to ./storage.
fn scan_and_extract(dev: &mut usb::FpDevice, label: &str) -> std::result::Result<Vec<u8>, String> {
    // 10 second timeout for finger placement
    let raw_frame = usb::scan(dev, 10_000).map_err(|e| format!("scan: {}", e))?;
    println!("  · Raw frame: {} bytes", raw_frame.len());

    let grayscale = image::deframe(&raw_frame).map_err(|e| format!("deframe: {}", e))?;
    println!(
        "  · Deframed image: {} bytes ({}×{})",
        grayscale.len(),
        usb::IMAGE_WIDTH,
        usb::IMAGE_HEIGHT
    );

    // Save debug PNG to ./storage for visual inspection.
    let storage_dir = Path::new("storage");
    if let Err(e) = std::fs::create_dir_all(storage_dir) {
        eprintln!("  (could not create storage directory: {})", e);
    }
    let png_path = storage_dir.join(format!("fp_debug_{}.png", label));
    match image::encode_png(
        &grayscale,
        usb::IMAGE_WIDTH as u32,
        usb::IMAGE_HEIGHT as u32,
    ) {
        Ok(png_bytes) => {
            if let Err(e) = std::fs::write(&png_path, &png_bytes) {
                eprintln!("  (could not save debug image: {})", e);
            } else {
                println!("  · Debug image saved: {}", png_path.display());
            }
        }
        Err(e) => eprintln!("  (could not encode debug PNG: {})", e),
    }

    let template = biometric::extract(&grayscale).map_err(|e| format!("extract: {}", e))?;
    Ok(template)
}

fn match_threshold_from_env() -> f64 {
    match std::env::var("FP_MATCH_THRESHOLD") {
        Ok(v) => v
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|t| (0.0..=1.0).contains(t))
            .unwrap_or(DEFAULT_MATCH_THRESHOLD),
        Err(_) => DEFAULT_MATCH_THRESHOLD,
    }
}
