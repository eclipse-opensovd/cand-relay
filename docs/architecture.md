<!--
SPDX-FileCopyrightText: 2026 Copyright (c) Contributors to cand-relay

See the NOTICE file(s) distributed with this work for additional
information regarding copyright ownership.

This program and the accompanying materials are made available under the
terms of the Apache License Version 2.0 which is available at
https://www.apache.org/licenses/LICENSE-2.0

SPDX-License-Identifier: Apache-2.0
-->

# Architecture

## Adding a new CAN backend

1. Create a new crate `cand-relay-backend-<name>`, depending on `cand-relay-core`.
2. Implement `cand_relay_core::CanBackend` for a struct wrapping whatever
   driver/library/socket you need. The trait is intentionally minimal:

   ```rust
   #[async_trait::async_trait]
   pub trait CanBackend: Send + Sync {
       async fn open(&mut self, params: &ChannelParams) -> Result<(), BackendError>;
       async fn send(&mut self, frame: RawCanFrame) -> Result<(), BackendError>;
       async fn recv(&mut self) -> Result<RawCanFrame, BackendError>;
       async fn close(&mut self) -> Result<(), BackendError>;
   }
   ```

3. Gate any platform- or vendor-SDK-specific code behind a Cargo feature
   (and `cfg(target_os = ...)` where relevant), following
   `cand-relay-backend-vector`'s `windows`/`linux` features or
   `cand-relay-backend-socketcan`'s `cfg(target_os = "linux")` split. A crate
   should always compile even when its hardware-specific feature is
   disabled -- fall back to a stub that returns `BackendError::Unavailable`.
4. Wire the new backend into `cand-relay`'s `Cargo.toml` (as an optional
   dependency gated by a new feature) and `src/main.rs`'s
   `build_bus_handle` match arm.
5. Add unit tests for any protocol/format translation and, if possible, a
   CI-runnable integration test (see `cand-relay-backend-socketcan`, which can
   be exercised against a `vcan0` interface in CI, unlike hardware-bound
   backends).

Nothing in `cand-relay-core` (the socketcand protocol implementation or the TCP
server) needs to change to support a new backend.

## Protocol subset

`cand-relay-core::protocol` implements only what CDA's `tokio-socketcan-isotp`
client actually uses:

- Handshake: `< hi >` (server) -> `< open <bus> >` (client) -> `< ok >`
  (server) -> `< rawmode >` (client, no reply expected).
- Steady state: `< frame <id> <sec.usec> <hexdata> >` (server -> client) and
  `< send <id> <dlc> <bytes...> >` (client -> server).

No BCM filters, ISO-TP framing, CAN-FD handshake, or `< close >` handling are
implemented -- ISO-TP segmentation happens entirely client-side in CDA, and
the bridge only ever needs to relay raw frames.

## Concurrency model

Each configured bus gets exactly one long-running task that owns the
`CanBackend` instance (see `cand_relay_core::server::spawn_bus`). Frames received
from the backend are broadcast to every connected client on that bus;
frames sent by any client are forwarded to the backend via an mpsc channel.
Multiple TCP clients can connect to the same bus concurrently.

## Vendor SDK acquisition (Vector XL-API)

The `cand-relay-backend-vector` crate never bundles, fetches, or auto-installs
Vector's XL Driver Library. This is a deliberate, license-driven decision,
not just a convenience choice -- see the "Obtaining the Vector XL Driver
Library SDK" section in the top-level `README.md` for the full rationale
(redistribution of the DLL/SO is prohibited by Vector's license without a
separate written agreement, and the download flow itself isn't scriptable).
Anyone building with the `windows`/`linux` feature must point `VXLAPI_H`/
`VXLAPI_LIB_DIR` at their own local install of the SDK.
