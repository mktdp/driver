//! Hardware integration tests.
//!
//! These tests require a physical U.are.U 4500 scanner to be plugged in.
//! They are gated behind the `hardware-tests` feature so CI can skip them.
//!
//! Run with:
//!   cargo test --features hardware-tests -- --test-threads=1
//!
//! IMPORTANT: --test-threads=1 because only one test can own the USB
//! device at a time.

use fingerprint_driver::{biometric, image, usb};

/// Test that we can open and close the device without panicking.
#[test]
#[cfg(feature = "hardware-tests")]
fn hw_open_close() {
    let dev = usb::open().expect("failed to open scanner — is it plugged in?");
    usb::close(dev);
}

/// Test that we can capture a raw frame from the device.
///
/// ⚠ Place your finger on the scanner before running this test!
#[test]
#[cfg(feature = "hardware-tests")]
fn hw_capture_raw_frame() {
    let mut dev = usb::open().expect("failed to open scanner");

    // 15 second timeout — enough time to place finger.
    let raw_frame = usb::scan(&mut dev, 15_000)
        .expect("scan failed — did you place your finger on the sensor?");

    // Should have at least the header + some pixel data.
    assert!(
        raw_frame.len() > usb::IMAGE_HEADER_LEN,
        "raw frame too short: {} bytes",
        raw_frame.len()
    );

    println!("raw frame: {} bytes", raw_frame.len());
    usb::close(dev);
}

/// Test the full pipeline: capture → deframe → extract template.
///
/// ⚠ Place your finger on the scanner before running this test!
#[test]
#[cfg(feature = "hardware-tests")]
fn hw_extract_template() {
    let mut dev = usb::open().expect("failed to open scanner");

    let raw_frame = usb::scan(&mut dev, 15_000)
        .expect("scan failed — did you place your finger?");

    let grayscale = image::deframe(&raw_frame)
        .expect("deframe failed");

    assert_eq!(
        grayscale.len(),
        usb::IMAGE_WIDTH * usb::IMAGE_HEIGHT,
        "unexpected image dimensions"
    );

    let template = biometric::extract(&grayscale)
        .expect("template extraction failed");

    assert!(
        !template.is_empty(),
        "extracted template is empty — no minutiae found"
    );

    println!("template: {} bytes", template.len());
    usb::close(dev);
}

/// Test that two scans of the same finger produce matching templates.
///
/// ⚠ You need to scan the SAME finger TWICE.
///   - Place finger → wait for first scan → lift finger
///   - Place SAME finger again → wait for second scan
///
/// Run this test alone:
///   cargo test --features hardware-tests hw_same_finger_match -- --test-threads=1
#[test]
#[cfg(feature = "hardware-tests")]
fn hw_same_finger_match() {
    let mut dev = usb::open().expect("failed to open scanner");

    println!("\n>>> Place your finger on the scanner for FIRST scan...");
    let tmpl_a = capture_template(&mut dev);
    println!("  Template A: {} bytes", tmpl_a.len());

    // Give user time to lift and re-place finger.
    println!(">>> Lift finger... waiting 3 seconds...");
    std::thread::sleep(std::time::Duration::from_secs(3));

    println!(">>> Place the SAME finger for SECOND scan...");
    let tmpl_b = capture_template(&mut dev);
    println!("  Template B: {} bytes", tmpl_b.len());

    let score = biometric::verify(&tmpl_a, &tmpl_b)
        .expect("verification failed");

    println!("  Similarity score: {:.4}", score);
    assert!(
        score > 0.05,
        "same-finger score too low: {:.4} — expected > 0.05",
        score
    );

    usb::close(dev);
}

/// Helper: scan + deframe + extract in one call.
#[cfg(feature = "hardware-tests")]
fn capture_template(dev: &mut usb::FpDevice) -> Vec<u8> {
    let raw = usb::scan(dev, 15_000).expect("scan failed");
    let gray = image::deframe(&raw).expect("deframe failed");
    biometric::extract(&gray).expect("extract failed")
}
