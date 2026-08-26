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

//! Backend-agnostic core of the `cand-relay`: a `socketcand`-wire-compatible
//! TCP server that relays raw CAN frames between a network client (e.g. the
//! Classic Diagnostic Adapter's `can-socketcand` transport) and a pluggable
//! [`CanBackend`] implementation.
//!
//! This crate intentionally implements only the raw-mode protocol subset: the
//! `hi` / `open` / `ok` / `rawmode` handshake, `frame` / `send` raw-frame
//! relay messages, and the CAN FD raw mode extension `fdframe` / `fdsend`
//! (linux-can/socketcand PR #79). No BCM filters or ISO-TP framing are part
//! of the wire protocol; ISO-TP segmentation happens entirely client-side.
//!
//! CAN FD: a bus configured with `[bus.fd]` runs its controller in FD mode.
//! Received FD frames are relayed as `< fdframe >` (with BRS/ESI flags);
//! classic frames keep using `< frame >`. Client `< fdsend >` frames are put
//! on the bus as FD frames with exactly the requested BRS flag (rejected on
//! non-FD buses), while classic `< send >` frames are transmitted per the
//! configured [`FdTxMode`].

pub mod backend;
pub mod config;
pub mod protocol;
pub mod server;

pub use backend::{BackendError, CanBackend, ChannelParams, FdParams, FdTxMode, RawCanFrame};
pub use config::{BridgeConfig, BusConfig, FdConfig, ServerConfig, WakeupConfig};
pub use server::{BusRegistry, run_server};
