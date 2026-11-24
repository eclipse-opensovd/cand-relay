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

//! An in-memory fake [`CanBackend`] with no hardware dependency, used for
//! `cand-relay-core` integration tests and for validating protocol compatibility
//! against a real CDA instance without needing CAN hardware in CI.

use async_trait::async_trait;
use cand_relay_core::{BackendError, CanBackend, ChannelParams, RawCanFrame};
use tokio::sync::mpsc;

/// A [`CanBackend`] driven entirely by channels: frames pushed into
/// `inbound` are returned by [`CanBackend::recv`], and frames passed to
/// [`CanBackend::send`] are forwarded to `sent`.
pub struct MockBackend {
    inbound: mpsc::Receiver<RawCanFrame>,
    sent: mpsc::Sender<RawCanFrame>,
}

/// The test-facing counterpart of a [`MockBackend`], used to script
/// "received" frames and observe frames the bridge sent to the bus.
pub struct MockBackendHandle {
    pub inbound: mpsc::Sender<RawCanFrame>,
    pub sent: mpsc::Receiver<RawCanFrame>,
}

/// Creates a connected [`MockBackend`] / [`MockBackendHandle`] pair.
#[must_use]
pub fn mock_backend() -> (MockBackend, MockBackendHandle) {
    let (inbound_tx, inbound_rx) = mpsc::channel(64);
    let (sent_tx, sent_rx) = mpsc::channel(64);
    (
        MockBackend {
            inbound: inbound_rx,
            sent: sent_tx,
        },
        MockBackendHandle {
            inbound: inbound_tx,
            sent: sent_rx,
        },
    )
}

/// The mock backend has no real devices to enumerate; any `selector` value
/// is accepted (and ignored) by [`MockBackend::open`]. Present so
/// `cand-relay list-devices --backend mock` has something sensible to
/// say instead of erroring.
#[must_use]
pub fn list_devices() -> Vec<String> {
    vec![
        "mock backend has no real devices; any selector value is accepted, e.g. selector = \
         \"mock0\""
            .to_string(),
    ]
}

#[async_trait]
impl CanBackend for MockBackend {
    async fn open(&mut self, _params: &ChannelParams) -> Result<(), BackendError> {
        Ok(())
    }

    async fn send(&mut self, frame: RawCanFrame) -> Result<(), BackendError> {
        self.sent
            .send(frame)
            .await
            .map_err(|e| BackendError::Send(e.to_string()))
    }

    async fn recv(&mut self) -> Result<RawCanFrame, BackendError> {
        self.inbound
            .recv()
            .await
            .ok_or_else(|| BackendError::Receive("mock backend closed".to_string()))
    }

    async fn close(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn relays_scripted_frame() {
        let (mut backend, handle) = mock_backend();
        let frame = RawCanFrame {
            id: 0x7E8,
            extended: false,
            data: vec![0x50, 0x01],
            ..Default::default()
        };
        handle.inbound.send(frame.clone()).await.unwrap();
        let received = backend.recv().await.unwrap();
        assert_eq!(received, frame);
    }

    #[tokio::test]
    async fn records_sent_frame() {
        let (mut backend, mut handle) = mock_backend();
        let frame = RawCanFrame {
            id: 0x7E0,
            extended: false,
            data: vec![0x3E, 0x00],
            ..Default::default()
        };
        backend.send(frame.clone()).await.unwrap();
        let recorded = handle.sent.recv().await.unwrap();
        assert_eq!(recorded, frame);
    }
}
