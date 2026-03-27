//! Build script:
//! 1. Fix `nbis-rs` `lib64 -> lib` symlink (Fedora/RHEL/openSUSE 64-bit).
//! 2. Generate `include/fingerprint.h` via cbindgen.

use std::{
    env, fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let target = env::var("TARGET").unwrap_or_default();

    // NOTE: keep this off by default. This build script runs after dependency
    // build scripts, so mutating `nbis-rs` here is too late for the current build
    // and can taint cached cargo checkouts for later builds.
    if target.contains("windows-msvc")
        && env::var_os("MKTDP_ENABLE_NBIS_PATCH_FROM_BUILD_RS").is_some()
    {
        patch_nbis_windows_msvc();
    }

    // Linux-only runtime linker fix for nbis-rs install layout.
    fix_nbis_lib64(&crate_dir);

    generate_header(&crate_dir);
}

fn generate_header(crate_dir: &str) {
    let config = cbindgen::Config::from_file("cbindgen.toml").unwrap_or_default();
    if let Ok(bindings) = cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_config(config)
        .generate()
    {
        let out_dir = Path::new(crate_dir).join("include");
        let _ = fs::create_dir_all(&out_dir);
        bindings.write_to_file(out_dir.join("fingerprint.h"));
    }
}

/// Scan the target build directory for nbis-rs output and create a
/// `lib -> lib64` symlink if needed.
fn fix_nbis_lib64(crate_dir: &str) {
    let out_dir = match env::var("OUT_DIR") {
        Ok(d) => d,
        Err(_) => return,
    };

    let build_parent = Path::new(&out_dir).parent().and_then(|p| p.parent());
    let build_dir = match build_parent {
        Some(d) => d,
        None => return,
    };

    let entries = match fs::read_dir(build_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("nbis-rs-") {
            continue;
        }

        let staging = entry.path().join("out/build/install_staging/nfiq2");
        if !staging.is_dir() {
            continue;
        }

        let lib64 = staging.join("lib64");
        let lib = staging.join("lib");
        if lib64.is_dir() && !lib.exists() {
            #[cfg(unix)]
            {
                let _ = std::os::unix::fs::symlink("lib64", &lib);
            }
        }
    }

    println!("cargo:rerun-if-changed={}", crate_dir);
}

fn patch_nbis_windows_msvc() {
    let Some(repo_dir) = find_latest_nbis_checkout() else {
        return;
    };

    let mut touched = Vec::new();

    let build_rs = repo_dir.join("build.rs");
    if patch_file(&build_rs, patch_nbis_build_rs_text) {
        touched.push(build_rs);
    }

    let superbuild = repo_dir.join("ext/NFIQ2-2.3.0/CMakeLists.txt");
    if patch_file(&superbuild, patch_nfiq2_superbuild_text) {
        touched.push(superbuild);
    }

    let compiler = repo_dir.join("ext/NFIQ2-2.3.0/cmake/compiler.cmake");
    if patch_file(&compiler, patch_nfiq2_compiler_text) {
        touched.push(compiler);
    }

    for path in touched {
        println!(
            "cargo:warning=Applied nbis-rs Windows compatibility patch: {}",
            path.display()
        );
    }
}

fn find_latest_nbis_checkout() -> Option<PathBuf> {
    let cargo_home = cargo_home()?;
    let checkouts = cargo_home.join("git").join("checkouts");
    let entries = fs::read_dir(checkouts).ok()?;

    let mut latest: Option<(SystemTime, PathBuf)> = None;
    for checkout in entries.flatten() {
        let checkout_name = checkout.file_name().to_string_lossy().to_string();
        if !checkout_name.starts_with("nbis-rs-") {
            continue;
        }

        let subdirs = match fs::read_dir(checkout.path()) {
            Ok(s) => s,
            Err(_) => continue,
        };
        for subdir in subdirs.flatten() {
            let candidate = subdir.path();
            if !candidate.join("build.rs").is_file() {
                continue;
            }
            let modified = subdir
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);

            match &latest {
                Some((current, _)) if modified <= *current => {}
                _ => latest = Some((modified, candidate)),
            }
        }
    }

    latest.map(|(_, path)| path)
}

fn cargo_home() -> Option<PathBuf> {
    if let Ok(path) = env::var("CARGO_HOME") {
        return Some(PathBuf::from(path));
    }
    if let Ok(user_profile) = env::var("USERPROFILE") {
        return Some(PathBuf::from(user_profile).join(".cargo"));
    }
    if let Ok(home) = env::var("HOME") {
        return Some(PathBuf::from(home).join(".cargo"));
    }
    None
}

fn patch_file(path: &Path, patcher: fn(&str) -> String) -> bool {
    if !path.is_file() {
        return false;
    }
    let original = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let has_crlf = original.contains("\r\n");
    let normalized = original.replace("\r\n", "\n");
    let patched = patcher(&normalized);
    if patched == normalized {
        return false;
    }

    let out = if has_crlf {
        patched.replace('\n', "\r\n")
    } else {
        patched
    };

    fs::write(path, out).is_ok()
}

fn patch_nbis_build_rs_text(input: &str) -> String {
    let mut out = input.to_string();

    let cfg_release = r#".define("CMAKE_CONFIGURATION_TYPES", "Release")"#;
    let cfg_dual = r#".define("CMAKE_CONFIGURATION_TYPES", "Debug;Release")"#;
    if out.contains(cfg_release) {
        out = out.replace(cfg_release, cfg_dual);
    }

    let build_type_define = r#".define("CMAKE_BUILD_TYPE", "Release")"#;
    if !out.contains(r#""CMAKE_CONFIGURATION_TYPES""#) && out.contains(build_type_define) {
        let replacement = format!(
            "{}\n        .define(\"CMAKE_CONFIGURATION_TYPES\", \"Debug;Release\")",
            build_type_define
        );
        out = out.replacen(build_type_define, &replacement, 1);
    }

    let old_link_block = r#"        println!("cargo:rustc-link-search=native=C:/msys64/mingw64/lib");
        println!("cargo:rustc-link-search=native={}/lib", &lib_src_dir_str);
        println!("cargo:rustc-link-search=native={}/x64/mingw/staticlib", &lib_src_dir_str);
        println!("cargo:rustc-link-lib=static=opencv_imgproc4100");
        println!("cargo:rustc-link-lib=static=opencv_ml4100");
        println!("cargo:rustc-link-lib=static=opencv_imgcodecs4100");
        println!("cargo:rustc-link-lib=static=opencv_core4100");
        println!("cargo:rustc-link-lib=static=FRFXLL_static");
        println!("cargo:rustc-link-lib=static=openblas");
        println!("cargo:rustc-link-lib=static=gomp");
        println!("cargo:rustc-link-lib=static=stdc++");"#;
    let new_link_block = r#"        // mktdp windows msvc patch: link against NFIQ2 packaged MSVC static libs.
        let profile = env::var("PROFILE").unwrap_or_else(|_| "release".to_string());
        let opencv_suffix = if profile == "debug" { "d" } else { "" };
        println!("cargo:rustc-link-search=native={}/lib", &lib_src_dir_str);
        println!("cargo:rustc-link-search=native={}/staticlib", &lib_src_dir_str);
        println!("cargo:rustc-link-lib=static=opencv_imgproc4100{}", opencv_suffix);
        println!("cargo:rustc-link-lib=static=opencv_ml4100{}", opencv_suffix);
        println!("cargo:rustc-link-lib=static=opencv_imgcodecs4100{}", opencv_suffix);
        println!("cargo:rustc-link-lib=static=opencv_core4100{}", opencv_suffix);
        println!("cargo:rustc-link-lib=static=FRFXLL_static");"#;
    if out.contains(old_link_block) {
        out = out.replace(old_link_block, new_link_block);
    }

    if !out.contains("mktdp windows zlib patch") {
        let zline = r#"    println!("cargo:rustc-link-lib=z");"#;
        let zpatch = r#"    // mktdp windows zlib patch: NBIS/OpenCV installs zlib as zlib(d).lib on MSVC.
    if target.contains("windows") {
        let profile = env::var("PROFILE").unwrap_or_else(|_| "release".to_string());
        let zlib_name = if profile == "debug" { "zlibd" } else { "zlib" };
        println!("cargo:rustc-link-lib=static={}", zlib_name);
    } else {
        println!("cargo:rustc-link-lib=z");
    }"#;
        if out.contains(zline) {
            out = out.replacen(zline, zpatch, 1);
        }
    }

    out
}

fn patch_nfiq2_superbuild_text(input: &str) -> String {
    let mut out = input.to_string();

    let safe_osx = r#"string(REPLACE ";" "$<SEMICOLON>" EXTERNALPROJECT_SAFE_OSX_ARCHITECTURES "${CMAKE_OSX_ARCHITECTURES}")"#;
    let safe_cfg = r#"string(REPLACE ";" "$<SEMICOLON>" EXTERNALPROJECT_SAFE_CMAKE_CONFIGURATION_TYPES "${CMAKE_CONFIGURATION_TYPES}")"#;
    if !out.contains(safe_cfg) && out.contains(safe_osx) {
        let replacement = format!("{safe_osx}\n{safe_cfg}");
        out = out.replacen(safe_osx, &replacement, 1);
    }

    let cfg_old = r#"-DCMAKE_CONFIGURATION_TYPES=${CMAKE_CONFIGURATION_TYPES}"#;
    let cfg_new =
        r#"-DCMAKE_CONFIGURATION_TYPES=${EXTERNALPROJECT_SAFE_CMAKE_CONFIGURATION_TYPES}"#;
    if out.contains(cfg_old) {
        out = out.replace(cfg_old, cfg_new);
    }

    let static_old = r#"-DBUILD_WITH_STATIC_CRT=ON"#;
    let static_new = r#"-DBUILD_WITH_STATIC_CRT=OFF"#;
    if out.contains(static_old) {
        out = out.replace(static_old, static_new);
    }

    out
}

fn patch_nfiq2_compiler_text(input: &str) -> String {
    let mut out = input.to_string();

    let runtime_block = r#"  # Static-link MS CRT
#  if (STATIC_LINK)
    foreach(flag_var
            CMAKE_C_FLAGS CMAKE_C_FLAGS_DEBUG CMAKE_C_FLAGS_RELEASE CMAKE_C_FLAGS_MINSIZEREL CMAKE_C_FLAGS_RELWITHDEBINFO
            CMAKE_CXX_FLAGS CMAKE_CXX_FLAGS_DEBUG CMAKE_CXX_FLAGS_RELEASE CMAKE_CXX_FLAGS_MINSIZEREL CMAKE_CXX_FLAGS_RELWITHDEBINFO)
      if(${flag_var} MATCHES "/MD")
        string(REGEX REPLACE "/MD" "/MT" ${flag_var} "${${flag_var}}")
      endif()
      if(${flag_var} MATCHES "/MDd")
        string(REGEX REPLACE "/MDd" "/MTd" ${flag_var} "${${flag_var}}")
      endif()
    endforeach(flag_var)
#  endif()"#;

    if out.contains(runtime_block) {
        out = out.replace(
            runtime_block,
            "  # mktdp windows msvc patch: keep /MD runtime to match Rust/OpenCV/FRFXLL linkage.",
        );
    }

    out
}
