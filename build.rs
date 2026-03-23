//! Build script:
//! 1. Fix nbis-rs lib64 → lib symlink (Fedora / RHEL / 64-bit distros)
//! 2. Generate `include/fingerprint.h` via cbindgen.

use std::path::Path;

fn main() {
    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();

    // ── Fix nbis-rs lib64 → lib symlink ────────────────────────────
    //
    // On 64-bit Linux distros (Fedora, RHEL, openSUSE, etc.) cmake
    // installs libraries into `lib64/` instead of `lib/`.  The
    // nbis-rs build.rs hardcodes `lib/` in its rustc-link-search
    // directives, so linking fails.
    //
    // We scan for the nbis-rs build output directory and create a
    // `lib → lib64` symlink if `lib64/` exists but `lib/` does not.
    fix_nbis_lib64(&crate_dir);

    // ── Generate C header ──────────────────────────────────────────

    let config = cbindgen::Config::from_file("cbindgen.toml")
        .unwrap_or_default();

    if let Ok(bindings) = cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
    {
        let out_dir = Path::new(&crate_dir).join("include");
        std::fs::create_dir_all(&out_dir).ok();
        bindings.write_to_file(out_dir.join("fingerprint.h"));
    }
}

/// Scan the target build directory for nbis-rs output and create a
/// `lib → lib64` symlink if needed.
fn fix_nbis_lib64(crate_dir: &str) {
    // Cargo sets OUT_DIR for *our* crate, e.g.:
    //   target/debug/build/fingerprint-driver-XXXX/out
    // We need to find nbis-rs's output which is a sibling:
    //   target/debug/build/nbis-rs-XXXX/out/build/install_staging/nfiq2/
    let out_dir = match std::env::var("OUT_DIR") {
        Ok(d) => d,
        Err(_) => return,
    };

    // Walk up from our OUT_DIR to the `build/` directory that contains
    // all crate build dirs.
    let build_parent = Path::new(&out_dir)
        .parent()  // fingerprint-driver-XXXX/
        .and_then(|p| p.parent()); // build/

    let build_dir = match build_parent {
        Some(d) => d,
        None => return,
    };

    // Find nbis-rs-*/out/build/install_staging/nfiq2/ directories.
    let entries = match std::fs::read_dir(build_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("nbis-rs-") {
            continue;
        }

        let staging = entry
            .path()
            .join("out/build/install_staging/nfiq2");

        if !staging.is_dir() {
            continue;
        }

        let lib64 = staging.join("lib64");
        let lib = staging.join("lib");

        // Create symlink only if lib64 exists and lib does not.
        if lib64.is_dir() && !lib.exists() {
            #[cfg(unix)]
            {
                let _ = std::os::unix::fs::symlink("lib64", &lib);
            }
        }
    }

    // Also tell Cargo to rerun if the staging dir changes.
    println!("cargo:rerun-if-changed={}", crate_dir);
}
