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

//! Raw FFI bindings to Vector's XL-API, generated at build time by
//! `build.rs` from the vendor-supplied `vxlapi.h` (see that file for the
//! required `VXLAPI_H` / `VXLAPI_LIB_DIR` environment variables). Nothing in
//! this module is hand-maintained; only the allow-list of symbols pulled in
//! lives in `build.rs`.
//!
//! This module is only compiled when the `windows` or `linux` feature is
//! enabled, since both require the vendor SDK to be present at build time.

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    clippy::all,
    reason = "generated bindings mirror the vendor C API verbatim"
)]

include!(concat!(env!("OUT_DIR"), "/vxlapi_bindings.rs"));
