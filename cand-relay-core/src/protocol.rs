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

//! `socketcand` wire protocol subset implementation.
//!
//! Only the messages actually sent/expected by the `tokio-socketcan-isotp`
//! client (used by the Classic Diagnostic Adapter's `can-socketcand`
//! transport) are implemented:
//!
//! Handshake (server -> client unless noted):
//! ```text
//! < hi >
//! < open <bus> >   (client -> server)
//! < ok >
//! < rawmode >      (client -> server, no reply expected)
//! ```
//!
//! Steady state:
//! ```text
//! < frame <id_hex> <sec.usec> <hexdata> >                (server -> client, RX classic frame)
//! < send <id_hex> <dlc> <b0> <b1> ... >                  (client -> server, TX frame)
//! < fdframe <id_hex> <flags> <sec.usec> <hexdata> >      (server -> client, RX CAN FD frame)
//! < fdsend <id_hex> <flags> <len> <b0> <b1> ... >        (client -> server, TX CAN FD frame)
//! ```
//!
//! The `fdframe` / `fdsend` messages follow the socketcand CAN FD raw mode
//! extension (linux-can/socketcand PR #79). `flags` is a hex value of
//! `CANFD_BRS` (0x01) and `CANFD_ESI` (0x02). Received CAN FD frames are
//! always relayed as `fdframe`; classic frames keep using `frame`.
//!
//! Framing is plain ASCII delimited by `<`/`>`, whitespace-tolerant, with no
//! length prefix or newline delimiter -- matching the client's own tolerant
//! byte-scanning reader.

use std::fmt::Write as _;

use crate::backend::RawCanFrame;

/// A parsed inbound message from a socketcand client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMessage {
    /// `< open <bus> >`
    Open { bus: String },
    /// `< rawmode >`
    RawMode,
    /// `< send <id> <dlc> <bytes...> >`
    Send(RawCanFrame),
    /// `< fdsend <id> <flags> <len> <bytes...> >`; the frame has `fd` set
    /// and its payload is zero-padded to the next valid CAN FD length.
    FdSend(RawCanFrame),
    /// Anything else we don't need to act on (ignored, matching the
    /// client's own tolerance of stray non-frame messages).
    Other(String),
}

/// Extracts the next complete `< ... >` message from `buf`, if any, and
/// returns the parsed message plus the number of bytes consumed from the
/// front of `buf`. Returns `None` if no complete message is buffered yet.
#[must_use]
pub fn parse_next_message(buf: &[u8]) -> Option<(ClientMessage, usize)> {
    let start = buf.iter().position(|&b| b == b'<')?;
    let rest = buf.get(start..)?;
    let len = rest.iter().position(|&b| b == b'>')?;
    let inner = std::str::from_utf8(rest.get(1..len)?).ok()?.trim();
    let consumed = start.checked_add(len)?.checked_add(1)?;

    let msg = parse_body(inner).unwrap_or_else(|| ClientMessage::Other(inner.to_string()));
    Some((msg, consumed))
}

fn parse_body(body: &str) -> Option<ClientMessage> {
    let mut parts = body.split_whitespace();
    let command = parts.next()?;
    match command {
        "open" => {
            let bus = parts.next()?.to_string();
            Some(ClientMessage::Open { bus })
        }
        "rawmode" => Some(ClientMessage::RawMode),
        "send" => {
            let id_str = parts.next()?;
            let id = u32::from_str_radix(id_str, 16).ok()?;
            // Extended IDs are conventionally sent zero-padded to 8 hex
            // digits by clients; anything longer than 3 hex digits or
            // exceeding the 11-bit range is treated as extended.
            let extended = id_str.len() > 3 || id > 0x7FF;
            let _dlc: usize = parts.next()?.parse().ok()?;
            let data = parts
                .map(|b| u8::from_str_radix(b, 16))
                .collect::<Result<Vec<u8>, _>>()
                .ok()?;
            // `fd` stays unset: payloads > 8 bytes are sent as FD by the
            // backend, shorter ones per the bus' configured `FdTxMode`.
            Some(ClientMessage::Send(RawCanFrame {
                id,
                extended,
                data,
                ..Default::default()
            }))
        }
        "fdsend" => {
            let id_str = parts.next()?;
            let id = u32::from_str_radix(id_str, 16).ok()?;
            // As for `send`, 8 hex digits select an extended identifier.
            let extended = id_str.len() == 8;
            if id > if extended { CAN_EFF_MASK } else { CAN_SFF_MASK } {
                return None;
            }
            let flags = u8::from_str_radix(parts.next()?, 16).ok()?;
            if flags & !(CANFD_BRS | CANFD_ESI) != 0 {
                return None;
            }
            let len: usize = parts.next()?.parse().ok()?;
            if len > CANFD_MAX_DLEN {
                return None;
            }
            let mut data = parts
                .map(|b| u8::from_str_radix(b, 16))
                .collect::<Result<Vec<u8>, _>>()
                .ok()?;
            if data.len() != len {
                return None;
            }
            data.resize(canfd_valid_len(len), 0);
            Some(ClientMessage::FdSend(RawCanFrame {
                id,
                extended,
                data,
                fd: true,
                brs: flags & CANFD_BRS != 0,
                esi: flags & CANFD_ESI != 0,
            }))
        }
        _ => None,
    }
}

/// CAN FD bit rate switch flag, as in Linux `CANFD_BRS`.
pub const CANFD_BRS: u8 = 0x01;
/// CAN FD error state indicator flag, as in Linux `CANFD_ESI`.
pub const CANFD_ESI: u8 = 0x02;
const CANFD_MAX_DLEN: usize = 64;
const CAN_SFF_MASK: u32 = 0x7FF;
const CAN_EFF_MASK: u32 = 0x1FFF_FFFF;

/// Rounds `len` up to the next valid CAN FD data length
/// (0..=8, 12, 16, 20, 24, 32, 48, 64).
fn canfd_valid_len(len: usize) -> usize {
    const VALID: [usize; 8] = [8, 12, 16, 20, 24, 32, 48, 64];
    if len <= 8 {
        return len;
    }
    VALID
        .into_iter()
        .find(|&v| len <= v)
        .unwrap_or(CANFD_MAX_DLEN)
}

/// Formats the initial greeting: `< hi >`.
#[must_use]
pub fn format_hi() -> String {
    "< hi >".to_string()
}

/// Formats the handshake acknowledgement: `< ok >`.
#[must_use]
pub fn format_ok() -> String {
    "< ok >".to_string()
}

/// Formats a received classic frame for relay to the client:
/// `< frame <id_hex> <sec.usec> <hexdata> >`.
#[must_use]
pub fn format_frame(frame: &RawCanFrame, sec: u64, usec: u64) -> String {
    let id_str = if frame.extended {
        format!("{:08X}", frame.id)
    } else {
        format!("{:X}", frame.id)
    };
    if frame.data.is_empty() {
        return format!("< frame {id_str} {sec}.{usec:06} >");
    }
    let data_hex = frame.data.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    });
    format!("< frame {id_str} {sec}.{usec:06} {data_hex} >")
}

/// Formats a received CAN FD frame for relay to the client:
/// `< fdframe <id_hex> <flags> <sec.usec> <HEXDATA> >`, matching
/// socketcand's CAN FD raw mode.
#[must_use]
pub fn format_fdframe(frame: &RawCanFrame, sec: u64, usec: u64) -> String {
    let id_str = if frame.extended {
        format!("{:08X}", frame.id)
    } else {
        format!("{:03X}", frame.id)
    };
    let mut flags = 0u8;
    if frame.brs {
        flags |= CANFD_BRS;
    }
    if frame.esi {
        flags |= CANFD_ESI;
    }
    let data_hex = frame
        .data
        .iter()
        .take(CANFD_MAX_DLEN)
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02X}");
            s
        });
    format!("< fdframe {id_str} {flags:02X} {sec}.{usec:06} {data_hex} >")
}

/// Formats a received frame as `fdframe` if it is a CAN FD frame, otherwise
/// as a classic `frame`.
#[must_use]
pub fn format_rx_frame(frame: &RawCanFrame, sec: u64, usec: u64) -> String {
    if frame.fd {
        format_fdframe(frame, sec, usec)
    } else {
        format_frame(frame, sec, usec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_open() {
        let (msg, consumed) = parse_next_message(b"< open can0 >").unwrap();
        assert_eq!(
            msg,
            ClientMessage::Open {
                bus: "can0".to_string()
            }
        );
        assert_eq!(consumed, 13);
    }

    #[test]
    fn parses_rawmode() {
        let (msg, _) = parse_next_message(b"< rawmode >").unwrap();
        assert_eq!(msg, ClientMessage::RawMode);
    }

    #[test]
    fn parses_send_standard_id() {
        let (msg, _) = parse_next_message(b"< send 7DF 2 3e 00 >").unwrap();
        match msg {
            ClientMessage::Send(frame) => {
                assert_eq!(frame.id, 0x7DF);
                assert!(!frame.extended);
                assert_eq!(frame.data, vec![0x3E, 0x00]);
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn parses_send_extended_id() {
        let (msg, _) = parse_next_message(b"< send 18DAF100 3 01 02 03 >").unwrap();
        match msg {
            ClientMessage::Send(frame) => {
                assert_eq!(frame.id, 0x18DA_F100);
                assert!(frame.extended);
                assert_eq!(frame.data, vec![0x01, 0x02, 0x03]);
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn parses_send_zero_length_data() {
        let (msg, _) = parse_next_message(b"< send 708 0 >").unwrap();
        match msg {
            ClientMessage::Send(frame) => {
                assert_eq!(frame.id, 0x708);
                assert!(frame.data.is_empty());
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn returns_none_on_incomplete_message() {
        assert!(parse_next_message(b"< open can0").is_none());
    }

    #[test]
    fn skips_leading_garbage_and_consumes_up_to_message_end() {
        let (_, consumed) = parse_next_message(b"garbage< rawmode >tail").unwrap();
        assert_eq!(
            &b"garbage< rawmode >tail"[..consumed],
            b"garbage< rawmode >"
        );
    }

    #[test]
    fn formats_frame_standard_id() {
        let frame = RawCanFrame {
            id: 0x708,
            extended: false,
            data: vec![0x01, 0x02],
            ..Default::default()
        };
        assert_eq!(format_frame(&frame, 1, 0), "< frame 708 1.000000 0102 >");
    }

    #[test]
    fn formats_frame_extended_id() {
        let frame = RawCanFrame {
            id: 0x18DA_F100,
            extended: true,
            data: vec![],
            ..Default::default()
        };
        assert_eq!(format_frame(&frame, 2, 123), "< frame 18DAF100 2.000123 >");
    }

    #[test]
    fn send_does_not_force_fd() {
        let (msg, _) = parse_next_message(b"< send 7E0 2 3e 00 >").unwrap();
        let ClientMessage::Send(frame) = msg else {
            panic!("unexpected message: {msg:?}");
        };
        assert!(!frame.fd && !frame.brs && !frame.esi);
    }

    #[test]
    fn parses_fdsend_pr79_example() {
        let input = b"< fdsend 123 01 12 11 22 33 44 55 66 77 88 99 aa bb cc >";
        let (msg, consumed) = parse_next_message(input).unwrap();
        assert_eq!(consumed, input.len());
        let ClientMessage::FdSend(frame) = msg else {
            panic!("unexpected message: {msg:?}");
        };
        assert_eq!(frame.id, 0x123);
        assert!(!frame.extended);
        assert!(frame.fd && frame.brs && !frame.esi);
        assert_eq!(
            frame.data,
            vec![
                0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC
            ]
        );
    }

    #[test]
    fn parses_fdsend_extended_esi_and_pads() {
        let (msg, _) = parse_next_message(b"< fdsend 18DAF110 02 9 1 2 3 4 5 6 7 8 9 >").unwrap();
        let ClientMessage::FdSend(frame) = msg else {
            panic!("unexpected message: {msg:?}");
        };
        assert_eq!(frame.id, 0x18DA_F110);
        assert!(frame.extended);
        assert!(frame.fd && !frame.brs && frame.esi);
        assert_eq!(frame.data, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 0, 0]);
    }

    #[test]
    fn parses_fdsend_short_and_empty() {
        let (msg, _) = parse_next_message(b"< fdsend 7E0 00 0 >").unwrap();
        let ClientMessage::FdSend(frame) = msg else {
            panic!("unexpected message: {msg:?}");
        };
        assert!(frame.fd && !frame.brs);
        assert!(frame.data.is_empty());

        let (msg, _) = parse_next_message(b"< fdsend 7E0 01 3 1 2 3 >").unwrap();
        let ClientMessage::FdSend(frame) = msg else {
            panic!("unexpected message: {msg:?}");
        };
        assert_eq!(frame.data, vec![1, 2, 3]);
    }

    #[test]
    fn rejects_invalid_fdsend() {
        for bad in [
            &b"< fdsend 123 04 1 00 >"[..], // unknown flag
            b"< fdsend 123 01 65 >",        // len > 64
            b"< fdsend 123 01 2 00 >",      // too few bytes
            b"< fdsend 123 01 1 00 11 >",   // too many bytes
            b"< fdsend 800 01 0 >",         // standard id out of range
            b"< fdsend 20000000 01 0 >",    // extended id out of range
            b"< fdsend 123 01 >",           // missing len
        ] {
            let (msg, _) = parse_next_message(bad).unwrap();
            assert!(
                matches!(msg, ClientMessage::Other(_)),
                "{} parsed as {msg:?}",
                String::from_utf8_lossy(bad)
            );
        }
    }

    #[test]
    fn canfd_valid_lengths() {
        let expected = [
            (0, 0),
            (8, 8),
            (9, 12),
            (12, 12),
            (13, 16),
            (21, 24),
            (25, 32),
            (33, 48),
            (49, 64),
            (64, 64),
        ];
        for (len, valid) in expected {
            assert_eq!(canfd_valid_len(len), valid, "len {len}");
        }
    }

    #[test]
    fn formats_fdframe_pr79_example() {
        let frame = RawCanFrame {
            id: 0x18DA_F110,
            extended: true,
            data: vec![
                0x00, 0x0A, 0x62, 0xF1, 0x90, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37,
            ],
            fd: true,
            brs: true,
            esi: false,
        };
        assert_eq!(
            format_fdframe(&frame, 23, 424_242),
            "< fdframe 18DAF110 01 23.424242 000A62F19031323334353637 >"
        );
    }

    #[test]
    fn formats_fdframe_standard_id_esi_empty() {
        let frame = RawCanFrame {
            id: 0x7E8,
            fd: true,
            esi: true,
            ..Default::default()
        };
        assert_eq!(format_fdframe(&frame, 1, 5), "< fdframe 7E8 02 1.000005  >");
    }

    #[test]
    fn rx_frame_selects_message_by_fd_flag() {
        let mut frame = RawCanFrame {
            id: 0x7E8,
            data: vec![0x01],
            ..Default::default()
        };
        assert!(format_rx_frame(&frame, 0, 0).starts_with("< frame "));
        frame.fd = true;
        assert!(format_rx_frame(&frame, 0, 0).starts_with("< fdframe "));
    }
}
