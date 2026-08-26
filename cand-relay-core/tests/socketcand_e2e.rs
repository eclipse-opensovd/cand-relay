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

//! End-to-end test of the socketcand-compatible server against a raw TCP
//! client, using the in-memory mock backend so no hardware/CAN interface is
//! needed. Validates the exact handshake and frame relay behaviour that
//! CDA's `tokio-socketcan-isotp`-based `can-socketcand` client relies on.

use std::time::Duration;

use cand_relay_backend_mock::{MockBackendHandle, mock_backend};
use cand_relay_core::{
    ChannelParams, FdParams, FdTxMode, RawCanFrame,
    server::{BusRegistry, run_server, spawn_bus},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};

async fn read_until(socket: &mut TcpStream, needle: &str) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 256];
    loop {
        let n = timeout(Duration::from_secs(2), socket.read(&mut chunk))
            .await
            .expect("timed out waiting for data")
            .expect("read error");
        assert!(n > 0, "connection closed unexpectedly");
        buf.extend_from_slice(chunk.get(..n).unwrap_or_default());
        let s = String::from_utf8_lossy(&buf);
        if s.contains(needle) {
            return s.to_string();
        }
    }
}

/// Starts a server with a single mock bus `can0` and returns a client
/// connected to it (handshake completed) plus the mock handle.
async fn connect(fd: Option<FdParams>) -> (TcpStream, MockBackendHandle) {
    let (backend, handle) = mock_backend();
    let bus_handle = spawn_bus(
        backend,
        ChannelParams {
            bitrate: 500_000,
            sample_point: None,
            selector: "0".to_string(),
            fd,
        },
    );

    let mut registry = BusRegistry::new();
    registry.insert("can0".to_string(), bus_handle);

    // Bind on an ephemeral port up front so we know it before the server
    // task starts accepting.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind failed");
    let addr = listener.local_addr().expect("local_addr failed");
    drop(listener);

    let listen = addr.to_string();
    tokio::spawn(async move {
        run_server(&listen, registry).await.expect("server error");
    });

    // Give the server a moment to actually start listening.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut socket = TcpStream::connect(addr).await.expect("connect failed");

    // --- Handshake ---
    read_until(&mut socket, "< hi >").await;
    socket
        .write_all(b"< open can0 >")
        .await
        .expect("write failed");
    read_until(&mut socket, "< ok >").await;
    socket
        .write_all(b"< rawmode >")
        .await
        .expect("write failed");

    (socket, handle)
}

async fn next_sent(handle: &mut MockBackendHandle) -> RawCanFrame {
    timeout(Duration::from_secs(2), handle.sent.recv())
        .await
        .expect("timed out waiting for relayed frame")
        .expect("mock backend channel closed")
}

#[tokio::test]
async fn full_handshake_and_frame_relay() {
    let (mut socket, mut handle) = connect(None).await;

    // --- Backend -> client relay ---
    handle
        .inbound
        .send(RawCanFrame {
            id: 0x7E8,
            extended: false,
            data: vec![0x50, 0x01],
            ..Default::default()
        })
        .await
        .expect("send failed");

    let received = read_until(&mut socket, "< frame").await;
    assert!(received.contains("< frame 7E8"));
    assert!(received.contains("5001"));

    // --- Client -> backend relay ---
    socket
        .write_all(b"< send 7E0 2 3e 00 >")
        .await
        .expect("write failed");

    let sent_frame = next_sent(&mut handle).await;
    assert_eq!(sent_frame.id, 0x7E0);
    assert_eq!(sent_frame.data, vec![0x3E, 0x00]);
}

#[tokio::test]
async fn canfd_fdframe_and_fdsend_relay() {
    let (mut socket, mut handle) = connect(Some(FdParams {
        data_bitrate: 2_000_000,
        data_sample_point: 75,
        iso: true,
        tx_mode: FdTxMode::FdBrs,
    }))
    .await;

    handle
        .inbound
        .send(RawCanFrame {
            id: 0x18DA_F110,
            extended: true,
            data: vec![
                0x00, 0x0A, 0x62, 0xF1, 0x90, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37,
            ],
            fd: true,
            brs: true,
            esi: false,
        })
        .await
        .expect("send failed");
    let received = read_until(&mut socket, " >").await;
    assert!(
        received.contains("< fdframe 18DAF110 01 ")
            && received.contains(" 000A62F19031323334353637 >"),
        "unexpected: {received}"
    );

    socket
        .write_all(b"< fdsend 123 00 9 1 2 3 4 5 6 7 8 9 >")
        .await
        .expect("write failed");
    let sent = next_sent(&mut handle).await;
    assert_eq!(sent.id, 0x123);
    assert!(sent.fd && !sent.brs && !sent.esi);
    assert_eq!(sent.data, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 0, 0]);
}

#[tokio::test]
async fn fdsend_rejected_on_classic_bus() {
    let (mut socket, mut handle) = connect(None).await;

    socket
        .write_all(b"< fdsend 123 01 1 aa >< send 7E0 1 bb >")
        .await
        .expect("write failed");
    // Only the classic frame must reach the backend.
    let sent = next_sent(&mut handle).await;
    assert_eq!(sent.id, 0x7E0);
    assert_eq!(sent.data, vec![0xBB]);
}
