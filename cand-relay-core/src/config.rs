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

//! Bridge configuration types, deserialized from a TOML file, e.g.:
//!
//! ```toml
//! [server]
//! listen = "0.0.0.0:29536"
//!
//! [[bus]]
//! name = "can0"
//! backend = "vector"
//! bitrate = 500000
//! selector = "0"
//! ```

use serde::Deserialize;

use crate::backend::{FdParams, FdTxMode};

/// The socketcand conventional default TCP port.
pub const DEFAULT_PORT: u16 = 29536;

#[derive(Debug, Clone, Deserialize)]
pub struct BridgeConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(rename = "bus", default)]
    pub buses: Vec<BusConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_listen")]
    pub listen: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: default_listen(),
        }
    }
}

fn default_listen() -> String {
    format!("0.0.0.0:{DEFAULT_PORT}")
}

/// A single logical CAN bus exposed by the bridge under a `bus` name that a
/// socketcand-compatible client selects via `< open <bus> >` (and CDA's
/// `socketcand:<host>:<port>:<bus>` address).
#[derive(Debug, Clone, Deserialize)]
pub struct BusConfig {
    /// The bus name clients use in `< open <bus> >` / the CDA address string.
    pub name: String,
    /// Which backend implementation handles this bus, e.g. "vector",
    /// "socketcan", or "mock". Interpreted by the binary's backend factory;
    /// unknown/disabled (feature-gated-out) backends are a configuration
    /// error at startup, not a compile error.
    pub backend: String,
    /// Nominal (arbitration phase) bitrate in bit/s.
    pub bitrate: u32,
    /// Nominal (arbitration phase) sample point in percent (e.g. `80`).
    /// Optional; if omitted the backend/driver picks one (for CAN FD buses
    /// the `vector` backend then defaults to 80%).
    #[serde(default)]
    pub sample_point: Option<u8>,
    /// Backend-specific channel selector (e.g. a Vector application channel
    /// index, or a `SocketCAN` interface name like "can0").
    pub selector: String,
    /// Optional CAN FD configuration. If present, the bus is opened in CAN
    /// FD mode (currently supported by the `vector` backend only).
    #[serde(default)]
    pub fd: Option<FdConfig>,
    /// Optional wake-up pulse to apply to a *separate* channel before/while
    /// this bus is used -- some ECUs are asleep by default and need a
    /// physical wake-up signal (e.g. Vector's "Activation Line"/"WakeUp
    /// Line", exposed as its own DAIO channel on devices like the
    /// VN5610A/VN5620) toggled before they'll respond on the actual CAN
    /// bus. See `list-devices` to find that channel.
    #[serde(default)]
    pub wakeup: Option<WakeupConfig>,
}

/// Configuration for a periodic/startup digital-output pulse used to wake
/// up a sleeping ECU, independent of (and in addition to) the CAN bus
/// itself. Only supported by backends that expose a digital I/O concept
/// (currently: `vector`, via its DAIO channels).
#[derive(Debug, Clone, Deserialize)]
pub struct WakeupConfig {
    /// Backend-specific selector for the channel carrying the wake-up
    /// line -- for the `vector` backend, the DAIO channel's index or name
    /// as reported by `cand-relay list-devices --backend vector`
    /// (e.g. `"VN5610A Channel 5"`, the VN5610A's Activation
    /// Line/WakeUp Line channel).
    pub selector: String,
    /// Bitmask of which digital output line(s) to pulse. Backend-specific;
    /// for `vector` these are `XL_DAIO_PORT_MASK_DIGITAL_D0`..`D7`
    /// (`0x01`..`0x80`) -- consult your device's manual for which bit maps
    /// to the Activation Line vs. `WakeUp` Line (commonly `D0` = Activation
    /// Line on VN5610A/VN5620). Defaults to `1` (`D0`).
    #[serde(default = "default_wakeup_port_mask")]
    pub port_mask: u32,
    /// How long to hold the line active/high, in milliseconds, before
    /// releasing it again. Defaults to 200ms.
    #[serde(default = "default_wakeup_pulse_ms")]
    pub pulse_ms: u64,
    /// If non-zero, repeat the pulse every this many seconds after the
    /// initial startup pulse (e.g. to keep an ECU that goes back to sleep
    /// reachable for the whole time the bridge runs). `0` (the default)
    /// means "pulse once at startup only".
    #[serde(default)]
    pub interval_secs: u64,
}

/// CAN FD settings for a bus, e.g.:
///
/// ```toml
/// [bus.fd]
/// data_bitrate = 2000000
/// data_sample_point = 75
/// tx_mode = "fd_brs"   # or "fd", "classic"
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct FdConfig {
    /// Data phase bitrate in bit/s.
    pub data_bitrate: u32,
    /// Data phase sample point in percent. Defaults to 75.
    #[serde(default = "default_data_sample_point")]
    pub data_sample_point: u8,
    /// ISO CAN FD (default `true`); `false` selects non-ISO (Bosch) CAN FD.
    #[serde(default = "default_true")]
    pub iso: bool,
    /// How client frames are put on the bus. Defaults to `"fd_brs"`.
    #[serde(default)]
    pub tx_mode: FdTxMode,
}

impl FdConfig {
    #[must_use]
    pub fn to_params(&self) -> FdParams {
        FdParams {
            data_bitrate: self.data_bitrate,
            data_sample_point: self.data_sample_point,
            iso: self.iso,
            tx_mode: self.tx_mode,
        }
    }
}

fn default_data_sample_point() -> u8 {
    75
}

fn default_true() -> bool {
    true
}

fn default_wakeup_port_mask() -> u32 {
    1
}

fn default_wakeup_pulse_ms() -> u64 {
    200
}

impl BridgeConfig {
    /// Parses a bridge configuration from TOML.
    ///
    /// # Errors
    ///
    /// Returns an error if `s` is not valid TOML or does not match the schema.
    pub fn from_toml_str(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config() {
        let cfg = BridgeConfig::from_toml_str(
            r#"
            [[bus]]
            name = "can0"
            backend = "mock"
            bitrate = 500000
            selector = "0"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.server.listen, "0.0.0.0:29536");
        assert_eq!(cfg.buses.len(), 1);
        assert_eq!(cfg.buses[0].name, "can0");
    }

    #[test]
    fn parses_fd_config() {
        let cfg = BridgeConfig::from_toml_str(
            r#"
            [[bus]]
            name = "can0"
            backend = "vector"
            bitrate = 500000
            sample_point = 80
            selector = "6"

            [bus.fd]
            data_bitrate = 1000000
            tx_mode = "fd"
            "#,
        )
        .unwrap();
        let bus = &cfg.buses[0];
        assert_eq!(bus.sample_point, Some(80));
        let fd = bus.fd.as_ref().unwrap().to_params();
        assert_eq!(fd.data_bitrate, 1_000_000);
        assert_eq!(fd.data_sample_point, 75);
        assert!(fd.iso);
        assert_eq!(fd.tx_mode, FdTxMode::Fd);
    }

    #[test]
    fn classic_config_has_no_fd() {
        let cfg = BridgeConfig::from_toml_str(
            r#"
            [[bus]]
            name = "can0"
            backend = "mock"
            bitrate = 500000
            selector = "0"
            "#,
        )
        .unwrap();
        assert!(cfg.buses[0].fd.is_none());
        assert!(cfg.buses[0].sample_point.is_none());
    }

    #[test]
    fn parses_custom_listen_address() {
        let cfg = BridgeConfig::from_toml_str(
            r#"
            [server]
            listen = "127.0.0.1:12345"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.server.listen, "127.0.0.1:12345");
    }
}
