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

//! The [`CanBackend`] abstraction: the single point every concrete CAN
//! hardware/driver integration (Vector XL-API, Linux `SocketCAN`, a mock, or
//! any future backend) must implement. Nothing in `cand-relay-core` or the wire
//! protocol depends on any specific vendor or platform.

use async_trait::async_trait;
use thiserror::Error;

/// A raw CAN (or CAN FD) frame, backend/protocol agnostic.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors the independent IDE/FDF/BRS/ESI bits of a CAN frame"
)]
pub struct RawCanFrame {
    /// CAN arbitration ID. Standard (11-bit) or extended (29-bit), as
    /// indicated by `extended`.
    pub id: u32,
    /// Whether `id` is a 29-bit extended identifier.
    pub extended: bool,
    /// Frame payload: 0-8 bytes for classic CAN, up to 64 bytes for CAN FD.
    pub data: Vec<u8>,
    /// Whether this is (RX) / should be sent as (TX) a CAN FD frame (FDF/EDL
    /// bit). On TX, a set `fd` is an explicit request (e.g. from a client
    /// `< fdsend >`): the frame is sent as CAN FD with exactly the given
    /// `brs`. Frames with `fd` unset (classic `< send >`) may be promoted to
    /// FD by backends running an FD-configured bus according to their
    /// configured [`FdTxMode`]; frames with more than 8 data bytes are
    /// always FD.
    pub fd: bool,
    /// CAN FD bit rate switch (BRS): data phase at the data bitrate. Only
    /// meaningful if `fd` is set.
    pub brs: bool,
    /// CAN FD error state indicator (ESI): the transmitting node is error
    /// passive. Only meaningful if `fd` is set. Reported on RX where the
    /// backend supports it; on TX it is passed through where the backend
    /// supports it and ignored otherwise.
    pub esi: bool,
}

/// How classic `< send >` frames submitted by clients (which carry no FD
/// flags, unlike `< fdsend >`) are transmitted on an FD-configured bus.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FdTxMode {
    /// Send every frame as CAN FD with bit rate switch (the usual choice
    /// for diagnostics on an FD bus, e.g. 500k/2M or 500k/1M).
    #[default]
    FdBrs,
    /// Send every frame as CAN FD without bit rate switch.
    Fd,
    /// Send frames of up to 8 bytes as classic CAN; only longer frames as
    /// CAN FD (with BRS).
    Classic,
}

/// CAN FD bus timing, used when a bus is configured for CAN FD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FdParams {
    /// Data phase bitrate in bit/s, e.g. `2_000_000`.
    pub data_bitrate: u32,
    /// Data phase sample point in percent, e.g. 75.
    pub data_sample_point: u8,
    /// ISO CAN FD (true, default) vs. non-ISO "Bosch" CAN FD (false).
    pub iso: bool,
    /// How client frames are transmitted, see [`FdTxMode`].
    pub tx_mode: FdTxMode,
}

/// Parameters used to open/activate a logical CAN channel on a backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelParams {
    /// Nominal (arbitration phase) bitrate in bit/s, e.g. `500_000`.
    pub bitrate: u32,
    /// Nominal (arbitration phase) sample point in percent, e.g. 80. `None`
    /// leaves the choice to the backend/driver.
    pub sample_point: Option<u8>,
    /// Backend-specific selector (e.g. a Vector XL-API application channel
    /// index, or a `SocketCAN` interface name). Interpreted by the backend
    /// implementation only.
    pub selector: String,
    /// CAN FD configuration; `None` means classic CAN only.
    pub fd: Option<FdParams>,
}

/// Error type returned by [`CanBackend`] operations.
#[derive(Debug, Error)]
pub enum BackendError {
    #[error("failed to open CAN channel: {0}")]
    Open(String),
    #[error("failed to send CAN frame: {0}")]
    Send(String),
    #[error("failed to receive CAN frame: {0}")]
    Receive(String),
    #[error("backend is not available on this platform/build: {0}")]
    Unavailable(String),
}

/// Abstraction over a single logical CAN channel on some hardware/driver.
///
/// Implementations exist per vendor/platform (e.g. `cand-relay-backend-vector`
/// for Vector's XL-API, `cand-relay-backend-socketcan` for Linux `SocketCAN`,
/// `cand-relay-backend-mock` for tests) and are selected purely by configuration
/// and Cargo feature flags; `cand-relay-core` never references a concrete
/// backend type.
#[async_trait]
pub trait CanBackend: Send + Sync {
    /// Open/activate the channel described by `params`. Must be callable
    /// again after [`CanBackend::close`] to support client reconnects.
    async fn open(&mut self, params: &ChannelParams) -> Result<(), BackendError>;

    /// Send a single raw frame on the bus.
    async fn send(&mut self, frame: RawCanFrame) -> Result<(), BackendError>;

    /// Wait for and return the next received frame on the bus.
    async fn recv(&mut self) -> Result<RawCanFrame, BackendError>;

    /// Close/deactivate the channel. Should not error if already closed.
    async fn close(&mut self) -> Result<(), BackendError>;
}
