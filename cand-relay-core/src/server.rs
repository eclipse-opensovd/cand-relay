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

//! The socketcand-compatible TCP server: connection handling, handshake, and
//! per-bus frame relay. Fully backend-agnostic -- callers hand in already
//! constructed [`CanBackend`] instances (via [`spawn_bus`]) keyed by bus
//! name; the server never constructs a backend itself.

use std::{
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};

use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{broadcast, mpsc},
};
use tracing::{debug, info, warn};

use crate::{
    backend::{CanBackend, ChannelParams, RawCanFrame},
    protocol::{ClientMessage, format_hi, format_ok, format_rx_frame, parse_next_message},
};

const FRAME_BROADCAST_CAPACITY: usize = 256;
const SEND_QUEUE_CAPACITY: usize = 256;
const READ_CHUNK_SIZE: usize = 512;

/// Per-bus channels connecting the network-facing relay loop to the
/// long-running task that owns the backend instance for that bus.
#[derive(Clone)]
pub struct BusHandle {
    /// Frames received from the backend, fanned out to every connected
    /// client.
    rx: broadcast::Sender<RawCanFrame>,
    /// Frames to be sent to the backend, submitted by any connected client.
    tx: mpsc::Sender<RawCanFrame>,
    /// Whether the bus is configured for CAN FD; `< fdsend >` is rejected
    /// otherwise (as socketcand does on non-FD interfaces).
    fd: bool,
}

/// Maps bus names (as used in `< open <bus> >`) to their [`BusHandle`].
pub type BusRegistry = HashMap<String, BusHandle>;

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Spawns the long-running task that owns `backend` for a single bus,
/// opening it with `params` and relaying frames to/from the returned
/// [`BusHandle`]. Call once per configured bus; register the handle in a
/// [`BusRegistry`] passed to [`run_server`].
pub fn spawn_bus<B>(mut backend: B, params: ChannelParams) -> BusHandle
where
    B: CanBackend + 'static,
{
    let (broadcast_tx, _) = broadcast::channel(FRAME_BROADCAST_CAPACITY);
    let (send_tx, mut send_rx) = mpsc::channel::<RawCanFrame>(SEND_QUEUE_CAPACITY);

    let handle = BusHandle {
        rx: broadcast_tx.clone(),
        tx: send_tx,
        fd: params.fd.is_some(),
    };

    tokio::spawn(async move {
        if let Err(e) = backend.open(&params).await {
            warn!("failed to open CAN backend for bus: {e}");
            return;
        }

        loop {
            tokio::select! {
                incoming = backend.recv() => {
                    match incoming {
                        Ok(frame) => {
                            // Ignore send errors: no subscribers is fine.
                            let _ = broadcast_tx.send(frame);
                        }
                        Err(e) => {
                            warn!("error receiving from CAN backend: {e}");
                        }
                    }
                }
                Some(frame) = send_rx.recv() => {
                    if let Err(e) = backend.send(frame).await {
                        warn!("error sending to CAN backend: {e}");
                    }
                }
            }
        }
    });

    handle
}

/// Runs the socketcand-compatible TCP server, accepting connections
/// indefinitely and relaying frames per the registered [`BusRegistry`].
///
/// # Errors
///
/// Returns an error if binding `listen` or accepting a connection fails.
pub async fn run_server(listen: &str, registry: BusRegistry) -> Result<(), ServerError> {
    let listener = TcpListener::bind(listen).await?;
    info!("cand-relay listening on {listen}");

    loop {
        let (socket, peer) = listener.accept().await?;
        let registry = registry.clone();
        tokio::spawn(async move {
            debug!("accepted connection from {peer}");
            if let Err(e) = handle_connection(socket, &registry).await {
                debug!("connection from {peer} closed: {e}");
            }
        });
    }
}

async fn handle_connection(
    mut socket: TcpStream,
    registry: &BusRegistry,
) -> Result<(), ServerError> {
    socket.write_all(format_hi().as_bytes()).await?;

    let mut buf = Vec::new();
    let mut chunk = [0u8; READ_CHUNK_SIZE];

    // --- Handshake: wait for `< open <bus> >`, then `< rawmode >`. ---
    let bus = loop {
        let n = socket.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(chunk.get(..n).unwrap_or_default());

        if let Some((msg, consumed)) = parse_next_message(&buf) {
            buf.drain(..consumed);
            if let ClientMessage::Open { bus } = msg {
                break bus;
            }
            // Ignore anything else pre-handshake, matching client tolerance.
        }
    };

    let Some(handle) = registry.get(&bus) else {
        warn!("client requested unknown bus '{bus}'");
        return Ok(());
    };
    socket.write_all(format_ok().as_bytes()).await?;

    // `< rawmode >` has no reply; consume it opportunistically along with
    // whatever else is already buffered, then fall through to steady state
    // regardless (matches the client, which does not wait for a reply).
    while let Some((msg, consumed)) = parse_next_message(&buf) {
        buf.drain(..consumed);
        if matches!(msg, ClientMessage::RawMode) {
            break;
        }
    }

    // --- Steady state: relay frames both ways. ---
    let mut frame_rx = handle.rx.subscribe();
    let send_tx = handle.tx.clone();

    loop {
        tokio::select! {
            result = socket.read(&mut chunk) => {
                let n = result?;
                if n == 0 {
                    return Ok(());
                }
                buf.extend_from_slice(chunk.get(..n).unwrap_or_default());
                while let Some((msg, consumed)) = parse_next_message(&buf) {
                    buf.drain(..consumed);
                    match msg {
                        ClientMessage::Send(frame) => {
                            let _ = send_tx.send(frame).await;
                        }
                        ClientMessage::FdSend(frame) if handle.fd => {
                            let _ = send_tx.send(frame).await;
                        }
                        ClientMessage::FdSend(_) => {
                            warn!("CAN FD is not supported by bus '{bus}', ignoring fdsend");
                        }
                        _ => {}
                    }
                }
            }
            frame = frame_rx.recv() => {
                match frame {
                    Ok(frame) => {
                        let (sec, usec) = now_sec_usec();
                        let msg = format_rx_frame(&frame, sec, usec);
                        socket.write_all(msg.as_bytes()).await?;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // Client is too slow; drop frames and continue.
                    }
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }
        }
    }
}

fn now_sec_usec() -> (u64, u64) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    (now.as_secs(), u64::from(now.subsec_micros()))
}
