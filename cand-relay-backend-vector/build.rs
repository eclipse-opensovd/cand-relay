/*
 * SPDX-FileCopyrightText: 2026 Copyright (c) Contributors to cand-relay
 *
 * See the NOTICE file(s) distributed with this work for additional
 * information regarding copyright ownership.
 *
 * This program and the accompanying materials are made available under the
 * terms of the Apache License Version 2.0 which is available at
 * https://www.apache.org/licenses/LICENSE-2.0
 *
 * SPDX-License-Identifier: Apache-2.0
 */

//! Generates FFI bindings to Vector's XL-API from the vendor SDK header
//! (`vxlapi.h`), rather than hand-maintaining struct layouts: the SDK is not
//! redistributable, so the header path must be supplied by whoever builds
//! with the `windows`/`linux` feature via the `VXLAPI_H` environment
//! variable (e.g. `VXLAPI_H=C:\Users\<you>\...\XL_Driver_Library\include\vxlapi.h`).
//! If `VXLAPI_H` is not set, the default XL Driver Library install location
//! (`C:\Users\Public\Documents\Vector\XL Driver Library 25.20.14.0\bin\vxlapi.h`)
//! is used as a fallback if present; otherwise the build fails with an error
//! instructing the user to set `VXLAPI_H`.
//!
//! Without either feature enabled this build script does nothing, so the
//! workspace and all other backends build without requiring the vendor SDK
//! to be present at all.

fn main() {
    let vector_feature_enabled = std::env::var("CARGO_FEATURE_WINDOWS").is_ok()
        || std::env::var("CARGO_FEATURE_LINUX").is_ok();
    if vector_feature_enabled {
        #[cfg(any(feature = "windows", feature = "linux"))]
        generate_bindings();
    }
}

#[cfg(any(feature = "windows", feature = "linux"))]
fn generate_bindings() {
    println!("cargo:rerun-if-env-changed=VXLAPI_H");
    const DEFAULT_HEADER: &str =
        r"C:\Users\Public\Documents\Vector\XL Driver Library 25.20.14.0\bin\vxlapi.h";
    let header = match std::env::var("VXLAPI_H") {
        Ok(path) => path,
        Err(_) if std::path::Path::new(DEFAULT_HEADER).exists() => DEFAULT_HEADER.to_string(),
        Err(_) => panic!(
            "the `windows`/`linux` feature of cand-relay-backend-vector requires the VXLAPI_H \
             environment variable to point at Vector's vxlapi.h from the XL Driver Library SDK \
             (not redistributed with this repository; download from vector.com); the default \
             install location ({DEFAULT_HEADER}) was not found either"
        ),
    };
    println!("cargo:rerun-if-changed={header}");

    // vxlapi.h relies on the includer having already pulled in <windows.h>
    // for types such as HANDLE, so wrap it accordingly before handing it to
    // bindgen.
    let header_escaped = header.replace('\\', "\\\\");
    let wrapper_contents = if std::env::var("CARGO_FEATURE_WINDOWS").is_ok() {
        format!("#include <windows.h>\n#include \"{header_escaped}\"\n")
    } else {
        format!("#include \"{header_escaped}\"\n")
    };

    let bindings = bindgen::Builder::default()
        .header_contents("vxlapi_wrapper.h", &wrapper_contents)
        .allowlist_function("xl.*")
        .allowlist_type("XL.*|s_xl.*|e_XL.*")
        .allowlist_var("XL_.*|CANFD_CONFOPT_.*")
        // Several constants (e.g. XL_CAN_EV_TAG_*) are `#define`d with casts,
        // which bindgen only evaluates via clang's macro fallback.
        .clang_macro_fallback()
        .generate()
        .expect("failed to generate XL-API bindings from vxlapi.h");

    let out_path = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("vxlapi_bindings.rs"))
        .expect("failed to write XL-API bindings");

    // Link against the vendor import library / shared object. The directory
    // containing vxlapi64.lib (Windows) or libvxlapi.so (Linux) must be
    // supplied via VXLAPI_LIB_DIR; the library itself is part of Vector's
    // SDK/driver install and is not redistributed with this repository.
    println!("cargo:rerun-if-env-changed=VXLAPI_LIB_DIR");
    if let Ok(lib_dir) = std::env::var("VXLAPI_LIB_DIR") {
        println!("cargo:rustc-link-search=native={lib_dir}");
    }
    if std::env::var("CARGO_FEATURE_WINDOWS").is_ok() {
        println!("cargo:rustc-link-lib=dylib=vxlapi64");
    } else {
        println!("cargo:rustc-link-lib=dylib=xlapi");
    }
}
