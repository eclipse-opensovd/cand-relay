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

//! Digital-output "wake-up pulse" support, independent of CAN frame relay.
//!
//! Some ECUs default to a low-power sleep state and won't respond on the
//! CAN bus at all until woken up by a dedicated physical signal. On Vector
//! hardware this is commonly exposed as its own DAIO (Digital/Analog I/O)
//! channel carrying the device's "Activation Line" and/or "WakeUp Line"
//! (see `XL_TRANSCEIVER_TYPE_DAIO_AL_WU`/`_WITH_AL` in `vxlapi.h`; e.g. the
//! VN5610A/VN5620 expose this as a separate virtual channel, distinct from
//! their actual CAN channels -- see `cand-relay list-devices --backend
//! vector`, look for a channel with `canCap=false` and a DAIO-related bus
//! capability). [`WakeupPulser`] toggles one or more digital output bits on
//! such a channel high for a configurable duration, to be called once at
//! startup and/or on a repeating interval (see
//! `cand_relay_core::config::WakeupConfig`).

use crate::{
    channel::{RX_QUEUE_SIZE, VectorError, check, resolve_access_mask},
    ffi,
};

const APP_NAME: &[u8] = b"cand-relay-wakeup\0";

/// An opened DAIO channel used purely to pulse digital output line(s), kept
/// open for the lifetime of the bridge so repeated pulses don't pay the
/// full open/activate cost each time.
pub struct WakeupPulser {
    driver_opened: bool,
    port_handle: ffi::XLportHandle,
    access_mask: ffi::XLaccess,
    port_mask: u32,
}

impl WakeupPulser {
    /// Opens the DAIO channel identified by `selector` (same index/name
    /// resolution as the CAN backend's `selector`) and activates it as a
    /// DAIO bus, ready to pulse the digital output bits in `port_mask`
    /// (see `XL_DAIO_PORT_MASK_DIGITAL_*` in `vxlapi.h`; consult your
    /// device's manual for which bit is the Activation/WakeUp line).
    pub fn open(selector: &str, port_mask: u32) -> Result<Self, VectorError> {
        // Safety: follows the XL-API's documented open/activate sequence,
        // same as `VectorBackend::open`; pointers passed are to locals
        // that outlive the call.
        unsafe {
            check("xlOpenDriver", ffi::xlOpenDriver())?;

            let access_mask = match resolve_access_mask(selector) {
                Ok(mask) => mask,
                Err(e) => {
                    ffi::xlCloseDriver();
                    return Err(e);
                }
            };

            let mut permission_mask: ffi::XLaccess = access_mask;
            let mut port_handle: ffi::XLportHandle = 0;
            if let Err(e) = check(
                "xlOpenPort",
                ffi::xlOpenPort(
                    &raw mut port_handle,
                    APP_NAME.as_ptr().cast_mut().cast(),
                    access_mask,
                    &raw mut permission_mask,
                    RX_QUEUE_SIZE,
                    ffi::XL_INTERFACE_VERSION,
                    ffi::XL_BUS_TYPE_DAIO,
                ),
            ) {
                ffi::xlCloseDriver();
                return Err(e);
            }

            if let Err(e) = check(
                "xlActivateChannel",
                ffi::xlActivateChannel(port_handle, access_mask, ffi::XL_BUS_TYPE_DAIO, 0),
            ) {
                ffi::xlClosePort(port_handle);
                ffi::xlCloseDriver();
                return Err(e);
            }

            Ok(Self {
                driver_opened: true,
                port_handle,
                access_mask,
                port_mask,
            })
        }
    }

    /// Drives the configured digital output line(s) high, holds them for
    /// `high_ms`, then releases them (drives low) again. Blocks the
    /// calling thread for `high_ms` -- callers on an async runtime should
    /// run this via `tokio::task::spawn_blocking`.
    pub fn pulse(&self, high_ms: u64) -> Result<(), VectorError> {
        self.set_output(self.port_mask)?;
        std::thread::sleep(std::time::Duration::from_millis(high_ms));
        self.set_output(0)
    }

    fn set_output(&self, value_mask: u32) -> Result<(), VectorError> {
        let mut params = ffi::XLdaioDigitalParams {
            portMask: self.port_mask,
            valueMask: value_mask,
        };
        // Safety: `port_handle`/`access_mask` were established by a
        // successful `open()`, and `params` is a valid, fully initialized
        // XLdaioDigitalParams as required by xlIoSetDigitalOutput.
        unsafe {
            check(
                "xlIoSetDigitalOutput",
                ffi::xlIoSetDigitalOutput(self.port_handle, self.access_mask, &raw mut params),
            )
        }
    }
}

impl Drop for WakeupPulser {
    fn drop(&mut self) {
        // Safety: mirrors VectorBackend::close's shutdown sequence.
        unsafe {
            if self.port_handle != ffi::XL_INVALID_PORTHANDLE as ffi::XLportHandle {
                let _ = ffi::xlClosePort(self.port_handle);
            }
            if self.driver_opened {
                ffi::xlCloseDriver();
            }
        }
    }
}
