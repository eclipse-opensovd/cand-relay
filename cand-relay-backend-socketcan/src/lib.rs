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

//! Linux `SocketCAN` passthrough backend: relays raw frames between the
//! bridge and a real kernel `SocketCAN` interface (e.g. `can0`, `vcan0`).
//!
//! Useful for local development/testing without any vendor hardware, and as
//! a legitimate backend in its own right when the bridge runs on a Linux
//! host that already has a `SocketCAN`-capable adapter attached directly
//! (i.e. not a Vector device), letting that host's bus be reached over the
//! network the same way a Vector-backed bus is.
//!
//! Only available when compiled for Linux (`cfg(target_os = "linux")`);
//! `channel_params` selector is the `SocketCAN` interface name (e.g. `"can0"`).

#![cfg_attr(not(target_os = "linux"), allow(unused))]

use async_trait::async_trait;
use cand_relay_core::{BackendError, CanBackend, ChannelParams, RawCanFrame};

#[cfg(target_os = "linux")]
mod linux {
    use socketcan::{CanFrame, EmbeddedFrame, ExtendedId, Id, StandardId, tokio::CanSocket};

    use super::{BackendError, CanBackend, ChannelParams, RawCanFrame, async_trait};

    /// `SocketCAN`-backed [`CanBackend`] implementation.
    #[derive(Default)]
    pub struct SocketCanBackend {
        socket: Option<CanSocket>,
    }

    impl SocketCanBackend {
        #[must_use]
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl CanBackend for SocketCanBackend {
        async fn open(&mut self, params: &ChannelParams) -> Result<(), BackendError> {
            let socket = CanSocket::open(&params.selector)
                .map_err(|e| BackendError::Open(format!("{}: {e}", params.selector)))?;
            self.socket = Some(socket);
            Ok(())
        }

        async fn send(&mut self, frame: RawCanFrame) -> Result<(), BackendError> {
            let socket = self
                .socket
                .as_ref()
                .ok_or_else(|| BackendError::Send("channel not open".to_string()))?;
            let id: Id = if frame.extended {
                ExtendedId::new(frame.id)
                    .map(Id::Extended)
                    .ok_or_else(|| BackendError::Send("invalid extended CAN id".to_string()))?
            } else {
                u16::try_from(frame.id)
                    .ok()
                    .and_then(StandardId::new)
                    .map(Id::Standard)
                    .ok_or_else(|| BackendError::Send("invalid standard CAN id".to_string()))?
            };
            let can_frame = CanFrame::new(id, &frame.data)
                .ok_or_else(|| BackendError::Send("invalid CAN frame payload".to_string()))?;
            socket
                .write_frame(can_frame)
                .await
                .map_err(|e| BackendError::Send(e.to_string()))
        }

        async fn recv(&mut self) -> Result<RawCanFrame, BackendError> {
            let socket = self
                .socket
                .as_ref()
                .ok_or_else(|| BackendError::Receive("channel not open".to_string()))?;
            let frame = socket
                .read_frame()
                .await
                .map_err(|e| BackendError::Receive(e.to_string()))?;
            let (id, extended) = match frame.id() {
                Id::Standard(id) => (u32::from(id.as_raw()), false),
                Id::Extended(id) => (id.as_raw(), true),
            };
            Ok(RawCanFrame {
                id,
                extended,
                data: frame.data().to_vec(),
                ..Default::default()
            })
        }

        async fn close(&mut self) -> Result<(), BackendError> {
            self.socket = None;
            Ok(())
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::SocketCanBackend;

/// Lists Linux network interfaces that look like `SocketCAN` devices, to
/// help pick a `selector` value. **The `selector` value is simply the
/// interface name** (e.g. `selector = "can0"`), exactly as printed here --
/// no translation needed, unlike the Vector backend.
///
/// Detection is based on `/sys/class/net/*/type` == `ARPHRD_CAN` (`280`),
/// which is how the kernel marks CAN netdevices (real `SocketCAN` adapters,
/// `vcan`/virtual CAN, and `can-gw` alike). Bring interfaces up first with
/// `sudo ip link set can0 up type can bitrate 500000` (or `sudo ip link add
/// dev vcan0 type vcan; sudo ip link set up vcan0` for a virtual bus) --
/// this only lists devices, it doesn't configure them.
#[cfg(target_os = "linux")]
#[must_use]
pub fn list_devices() -> Vec<String> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir("/sys/class/net") {
        Ok(entries) => entries,
        Err(e) => return vec![format!("failed to read /sys/class/net: {e}")],
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let type_path = entry.path().join("type");
        let Ok(kind) = std::fs::read_to_string(&type_path) else {
            continue;
        };
        // ARPHRD_CAN, see linux/if_arp.h.
        if kind.trim() == "280" {
            let operstate = std::fs::read_to_string(entry.path().join("operstate"))
                .unwrap_or_else(|_| "unknown".to_string());
            out.push(format!(
                "{name} (operstate={}) -> selector=\"{name}\"",
                operstate.trim()
            ));
        }
    }
    if out.is_empty() {
        out.push(
            "no SocketCAN interfaces found under /sys/class/net (bring one up first, e.g. `sudo \
             ip link set can0 up type can bitrate 500000` or add a virtual one with `sudo ip link \
             add dev vcan0 type vcan && sudo ip link set up vcan0`)"
                .to_string(),
        );
    }
    out
}

/// Stub used on non-Linux platforms, so `cand-relay list-devices
/// --backend socketcan` still builds/links everywhere.
#[cfg(not(target_os = "linux"))]
pub fn list_devices() -> Vec<String> {
    vec!["SocketCAN backend is only available on Linux".to_string()]
}

/// Stub implementation on non-Linux platforms: always returns
/// [`BackendError::Unavailable`]. Present so the crate compiles on every
/// platform without callers needing platform-specific `cfg`s of their own;
/// the actual SocketCAN I/O is entirely feature/platform gated via
/// `cfg(target_os = "linux")` above.
#[cfg(not(target_os = "linux"))]
#[derive(Default)]
pub struct SocketCanBackend;

#[cfg(not(target_os = "linux"))]
impl SocketCanBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(not(target_os = "linux"))]
#[async_trait]
impl CanBackend for SocketCanBackend {
    async fn open(&mut self, _params: &ChannelParams) -> Result<(), BackendError> {
        Err(BackendError::Unavailable(
            "SocketCAN backend is only available on Linux".to_string(),
        ))
    }

    async fn send(&mut self, _frame: RawCanFrame) -> Result<(), BackendError> {
        Err(BackendError::Unavailable(
            "SocketCAN backend is only available on Linux".to_string(),
        ))
    }

    async fn recv(&mut self) -> Result<RawCanFrame, BackendError> {
        Err(BackendError::Unavailable(
            "SocketCAN backend is only available on Linux".to_string(),
        ))
    }

    async fn close(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
}
