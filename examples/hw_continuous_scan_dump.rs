//! Hardware test: continuous scanning dump.
//!
//! Keeps scanning and prints summary data for every scan until you
//! stop the process (Ctrl+C).
//!
//! Run with:
//!   cargo run --example hw_continuous_scan_dump --features hardware-tests

use std::ffi::CStr;
use std::os::raw::c_void;
use std::slice;

use mktdp_driver::{
    error::FP_OK, fp_close, fp_open, fp_scan_continuous, fp_strerror, FpTemplateCallback,
};

unsafe extern "C" fn on_template(ptr: *const u8, len: usize, user_data: *mut c_void) -> bool {
    if ptr.is_null() || len == 0 {
        eprintln!("scan callback received empty template");
        return true;
    }

    // SAFETY: callback contract guarantees pointer validity for callback duration.
    let bytes = unsafe { slice::from_raw_parts(ptr, len) };
    let count_ptr = user_data.cast::<u64>();
    if !count_ptr.is_null() {
        // SAFETY: caller passes a valid mutable counter pointer.
        unsafe {
            *count_ptr = (*count_ptr).saturating_add(1);
        }
    }

    let scan_no = if count_ptr.is_null() {
        0
    } else {
        // SAFETY: same as above; read-only access.
        unsafe { *count_ptr }
    };

    let preview_len = bytes.len().min(16);
    let mut preview = String::new();
    for b in &bytes[..preview_len] {
        if !preview.is_empty() {
            preview.push(' ');
        }
        preview.push_str(&format!("{:02x}", b));
    }

    println!(
        "[scan #{:04}] template_len={} bytes preview={}{}",
        scan_no,
        bytes.len(),
        preview,
        if bytes.len() > preview_len {
            " ..."
        } else {
            ""
        }
    );

    true
}

fn main() {
    println!("=== MKTDP Driver — Continuous Scan Dump ===\n");
    println!("Scanner will stay active and print one line per scan.");
    println!("Stop anytime with Ctrl+C.\n");

    let dev = fp_open();
    if dev.is_null() {
        eprintln!("failed to open scanner");
        std::process::exit(1);
    }

    let mut scan_count: u64 = 0;
    let user_data = (&mut scan_count as *mut u64).cast::<c_void>();
    let callback: FpTemplateCallback = on_template;

    let rc = fp_scan_continuous(
        dev, 10_000, // timeout per scan
        0,      // max_scans=0 => run until stopped
        callback, user_data,
    );

    if rc != FP_OK {
        let msg = unsafe { CStr::from_ptr(fp_strerror(rc)) }
            .to_str()
            .unwrap_or("unknown error");
        eprintln!("continuous scan ended with error {}: {}", rc, msg);
    }

    fp_close(dev);
}
