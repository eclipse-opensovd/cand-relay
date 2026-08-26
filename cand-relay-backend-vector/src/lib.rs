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

//! Vector XL-API [`CanBackend`](cand_relay_core::CanBackend) implementation.
//!
//! Requires the `windows` or `linux` feature (mutually exclusive with the
//! target OS they name) and the vendor SDK to be supplied at build time via
//! the `VXLAPI_H` / `VXLAPI_LIB_DIR` environment variables -- see
//! `build.rs`. Works across the entire Vector XL-API hardware line (the
//! same handful of function calls address any supported VN-series device);
//! device selection is purely a configuration concern (an application
//! channel index), not a code branch.
//!
//! Without either feature enabled, this crate still compiles and exposes a
//! [`VectorBackend`] stub that returns
//! [`BackendError::Unavailable`](cand_relay_core::BackendError::Unavailable),
//! so callers/binaries don't need their own platform `cfg`s.

#[cfg(any(feature = "windows", feature = "linux"))]
mod channel;
#[cfg(any(feature = "windows", feature = "linux"))]
mod ffi;
#[cfg(any(feature = "windows", feature = "linux"))]
mod wakeup;
// Pure (FFI-free) CAN FD timing helpers; compiled always so they are
// unit-testable without the vendor SDK.
#[cfg_attr(
    not(any(feature = "windows", feature = "linux")),
    allow(dead_code, reason = "only used by the XL-API backend")
)]
mod timing;

#[cfg(any(feature = "windows", feature = "linux"))]
pub use channel::{VectorBackend, VectorError};
#[cfg(any(feature = "windows", feature = "linux"))]
pub use wakeup::WakeupPulser;

/// Enumerates every XL-API channel visible on this machine via
/// `xlGetDriverConfig`, independent of any application-channel assignment
/// (Vector Hardware Manager setup is not required). Used by
/// `cand-relay list-devices --backend vector` to help pick a `selector`
/// value.
///
/// Each returned line corresponds to one XL-API channel and includes its
/// `channelIndex` -- **this is the value to put in the `selector` field**
/// of a `[[bus]]` entry using the `vector` backend, as a string (e.g.
/// `selector = "2"`). Only use channels where `canCap=true`; the others are
/// present on the same hardware but don't support classic CAN (e.g. LIN,
/// I/O channels, or CAN-FD-only channels reported separately).
#[cfg(any(feature = "windows", feature = "linux"))]
pub fn list_channels() -> Vec<String> {
    unsafe {
        ffi::xlOpenDriver();
        let mut cfg: ffi::XLdriverConfig = std::mem::zeroed();
        let status = ffi::xlGetDriverConfig(&raw mut cfg);
        let channel_count = { cfg.channelCount };
        let mut out = vec![format!(
            "xlGetDriverConfig status={status} channelCount={channel_count}"
        )];
        for i in 0..channel_count as usize {
            let ch = cfg.channel[i];
            let name = std::ffi::CStr::from_ptr(ch.name.as_ptr())
                .to_string_lossy()
                .into_owned();
            let (hw_type, hw_index, hw_channel, channel_mask, channel_index, is_on_bus, bus_caps) = (
                ch.hwType,
                ch.hwIndex,
                ch.hwChannel,
                ch.channelMask,
                ch.channelIndex,
                ch.isOnBus,
                ch.channelBusCapabilities,
            );
            out.push(format!(
                "[{i}] name={name:?} hwType={hw_type} hwIndex={hw_index} hwChannel={hw_channel} \
                 channelMask={channel_mask} channelIndex={channel_index} isOnBus={is_on_bus} \
                 busCaps={bus_caps:#010x} canCap={} fdCap={} -> selector=\"{channel_index}\"",
                bus_caps & ffi::XL_BUS_ACTIVE_CAP_CAN != 0,
                ch.channelCapabilities
                    & (ffi::XL_CHANNEL_FLAG_CANFD_ISO_SUPPORT
                        | ffi::XL_CHANNEL_FLAG_CANFD_BOSCH_SUPPORT)
                    != 0
            ));
        }
        ffi::xlCloseDriver();
        out
    }
}

/// Stub used when neither `windows` nor `linux` is enabled, so
/// `cand-relay list-devices --backend vector` still builds/links and
/// gives a clear error instead of the binary failing to compile.
#[cfg(not(any(feature = "windows", feature = "linux")))]
#[must_use]
pub fn list_channels() -> Vec<String> {
    vec![
        "cand-relay-backend-vector was built without the `windows`/`linux` feature; rebuild with \
         --features vector-windows (or vector-linux) to enumerate XL-API channels."
            .to_string(),
    ]
}

#[cfg(not(any(feature = "windows", feature = "linux")))]
mod stub {
    use async_trait::async_trait;
    use cand_relay_core::{BackendError, CanBackend, ChannelParams, RawCanFrame};

    #[derive(Default)]
    pub struct VectorBackend;

    impl VectorBackend {
        #[must_use]
        pub fn new() -> Self {
            Self
        }
    }

    #[async_trait]
    impl CanBackend for VectorBackend {
        async fn open(&mut self, _params: &ChannelParams) -> Result<(), BackendError> {
            Err(unavailable())
        }

        async fn send(&mut self, _frame: RawCanFrame) -> Result<(), BackendError> {
            Err(unavailable())
        }

        async fn recv(&mut self) -> Result<RawCanFrame, BackendError> {
            Err(unavailable())
        }

        async fn close(&mut self) -> Result<(), BackendError> {
            Ok(())
        }
    }

    fn unavailable() -> BackendError {
        BackendError::Unavailable(
            "cand-relay-backend-vector was built without the `windows`/`linux` feature".to_string(),
        )
    }

    /// Stub used when neither `windows` nor `linux` is enabled, mirroring
    /// [`VectorBackend`]'s stub above.
    pub struct WakeupPulser;

    impl WakeupPulser {
        /// # Errors
        ///
        /// Always returns [`super::VectorError::Unavailable`].
        pub fn open(_selector: &str, _port_mask: u32) -> Result<Self, super::VectorError> {
            Err(super::VectorError::Unavailable)
        }

        /// # Errors
        ///
        /// Always returns [`super::VectorError::Unavailable`].
        pub fn pulse(&self, _high_ms: u64) -> Result<(), super::VectorError> {
            Err(super::VectorError::Unavailable)
        }
    }
}

#[cfg(not(any(feature = "windows", feature = "linux")))]
pub use stub::WakeupPulser;

#[cfg(not(any(feature = "windows", feature = "linux")))]
#[derive(Debug, thiserror::Error)]
pub enum VectorError {
    #[error("cand-relay-backend-vector was built without the `windows`/`linux` feature")]
    Unavailable,
}

#[cfg(not(any(feature = "windows", feature = "linux")))]
pub use stub::VectorBackend;
