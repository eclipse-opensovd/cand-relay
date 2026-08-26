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

//! CAN FD bit timing calculation for `xlCanFdSetConfiguration`.
//!
//! The XL-API takes bitrate plus `sjw`/`tseg1`/`tseg2` (in time quanta) per
//! phase and derives the prescaler itself from the controller clock. Vector's
//! CAN FD capable devices clock their controllers at 80 MHz, so we pick a
//! common prescaler for both phases (as recommended for CAN FD) such that
//! each phase's bit is an integral number of time quanta within the
//! segment ranges the XL-API accepts, then split it per the requested
//! sample point. Kept free of FFI so it can be unit-tested without the SDK.

/// Controller clock of Vector CAN FD capable devices.
pub const CAN_FD_CLOCK_HZ: u32 = 80_000_000;

/// Conservative segment limits (in time quanta), within the documented
/// XL-API ranges for both phases.
const MAX_TSEG1_ABR: u32 = 254;
const MAX_TSEG2_ABR: u32 = 127;
const MAX_TSEG1_DBR: u32 = 31;
const MAX_TSEG2_DBR: u32 = 15;
const MIN_TQ: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseTiming {
    pub bitrate: u32,
    pub sjw: u32,
    pub tseg1: u32,
    pub tseg2: u32,
}

impl PhaseTiming {
    /// Total time quanta per bit (sync segment + tseg1 + tseg2).
    #[must_use]
    pub fn tq(&self) -> u32 {
        self.tseg1.saturating_add(self.tseg2).saturating_add(1)
    }

    /// Effective sample point in percent.
    #[must_use]
    pub fn sample_point(&self) -> f64 {
        f64::from(self.tseg1.saturating_add(1)) * 100.0 / f64::from(self.tq())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FdTiming {
    pub prescaler: u32,
    pub arbitration: PhaseTiming,
    pub data: PhaseTiming,
}

fn split(bitrate: u32, tq: u32, sample_point: u8) -> PhaseTiming {
    let sp = u32::from(sample_point.clamp(50, 95));
    // tseg2 = time after the sample point, rounded to the nearest quantum.
    let tseg2 = (tq
        .saturating_mul(100u32.saturating_sub(sp))
        .saturating_add(50)
        / 100)
        .max(1);
    // Saturates to 0 if `tq` is too small; rejected by the caller's limit check.
    let tseg1 = tq.saturating_sub(1).saturating_sub(tseg2);
    PhaseTiming {
        bitrate,
        sjw: tseg2.min(tseg1),
        tseg1,
        tseg2,
    }
}

/// Computes CAN FD timing for the given arbitration/data bitrates and
/// sample points (percent). Returns `None` if no prescaler yields an
/// integral number of time quanta for both phases within the limits.
#[must_use]
pub fn compute_fd_timing(
    arb_bitrate: u32,
    arb_sample_point: u8,
    data_bitrate: u32,
    data_sample_point: u8,
) -> Option<FdTiming> {
    if arb_bitrate == 0 || data_bitrate == 0 {
        return None;
    }
    // Smallest prescaler (= finest resolution) that fits the limits.
    (1..=256u32).find_map(|prescaler| {
        let arb_div = prescaler.checked_mul(arb_bitrate)?;
        let data_div = prescaler.checked_mul(data_bitrate)?;
        if !CAN_FD_CLOCK_HZ.is_multiple_of(arb_div) || !CAN_FD_CLOCK_HZ.is_multiple_of(data_div) {
            return None;
        }
        let arb_tq = CAN_FD_CLOCK_HZ.checked_div(arb_div)?;
        let data_tq = CAN_FD_CLOCK_HZ.checked_div(data_div)?;
        if arb_tq < MIN_TQ || data_tq < MIN_TQ {
            return None;
        }
        let arbitration = split(arb_bitrate, arb_tq, arb_sample_point);
        let data = split(data_bitrate, data_tq, data_sample_point);
        let fits = arbitration.tseg1 <= MAX_TSEG1_ABR
            && arbitration.tseg2 <= MAX_TSEG2_ABR
            && data.tseg1 <= MAX_TSEG1_DBR
            && data.tseg2 <= MAX_TSEG2_DBR
            && arbitration.tseg1 >= 1
            && data.tseg1 >= 1;
        fits.then_some(FdTiming {
            prescaler,
            arbitration,
            data,
        })
    })
}

/// Maps a CAN FD DLC code (0-15) to its payload length in bytes.
#[must_use]
pub fn dlc_to_len(dlc: u8) -> usize {
    match dlc {
        0..=8 => usize::from(dlc),
        9 => 12,
        10 => 16,
        11 => 20,
        12 => 24,
        13 => 32,
        14 => 48,
        _ => 64,
    }
}

/// Maps a payload length (0-64) to the smallest CAN FD DLC code that can
/// carry it.
#[must_use]
pub fn len_to_dlc(len: usize) -> u8 {
    match len {
        0..=8 => u8::try_from(len).unwrap_or(8),
        9..=12 => 9,
        13..=16 => 10,
        17..=20 => 11,
        21..=24 => 12,
        25..=32 => 13,
        33..=48 => 14,
        _ => 15,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_500k_80_1m_75() {
        let t = compute_fd_timing(500_000, 80, 1_000_000, 75).unwrap();
        assert_eq!(t.prescaler, 2);
        assert_eq!(t.arbitration.tq(), 80);
        assert_eq!(t.data.tq(), 40);
        assert!((t.arbitration.sample_point() - 80.0).abs() < 0.01);
        assert!((t.data.sample_point() - 75.0).abs() < 0.01);
        assert_eq!(t.arbitration.tseg1, 63);
        assert_eq!(t.arbitration.tseg2, 16);
        assert_eq!(t.data.tseg1, 29);
        assert_eq!(t.data.tseg2, 10);
    }

    #[test]
    fn timing_500k_2m() {
        let t = compute_fd_timing(500_000, 80, 2_000_000, 70).unwrap();
        assert_eq!(t.data.tq() * 2_000_000 * t.prescaler, CAN_FD_CLOCK_HZ);
        assert_eq!(t.arbitration.tq() * 500_000 * t.prescaler, CAN_FD_CLOCK_HZ);
        assert!((t.data.sample_point() - 70.0).abs() < 0.01);
    }

    #[test]
    fn timing_500k_5m() {
        let t = compute_fd_timing(500_000, 80, 5_000_000, 75).unwrap();
        assert_eq!(t.data.tq() * 5_000_000 * t.prescaler, CAN_FD_CLOCK_HZ);
    }

    #[test]
    fn timing_rejects_impossible_rates() {
        assert!(compute_fd_timing(500_000, 80, 3_000_001, 75).is_none());
        assert!(compute_fd_timing(0, 80, 1_000_000, 75).is_none());
    }

    #[test]
    fn dlc_roundtrip() {
        for len in 0..=64 {
            let dlc = len_to_dlc(len);
            assert!(dlc_to_len(dlc) >= len);
            if dlc > 0 {
                assert!(dlc_to_len(dlc - 1) < len);
            }
        }
    }
}
