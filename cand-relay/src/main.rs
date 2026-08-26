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

//! `cand-relay` binary: reads a TOML config, constructs the configured
//! backend for each bus (only backends compiled in via Cargo features are
//! available), and runs the socketcand-compatible server.

use std::path::PathBuf;

use cand_relay_core::{
    BridgeConfig, ChannelParams, WakeupConfig,
    server::{BusRegistry, run_server, spawn_bus},
};
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "cand-relay", about = "socketcand-compatible CAN bridge")]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the bridge server (default if no subcommand is given).
    Run {
        /// Path to the bridge TOML configuration file.
        #[arg(short, long, default_value = "cand-relay.toml")]
        config: PathBuf,
    },
    /// List CAN devices/channels a backend can see, and how to turn each
    /// one into a `selector` value for `cand-relay.toml`.
    ///
    /// Run this once per backend/machine before writing a config: each
    /// backend addresses hardware differently (Vector's XL-API channel
    /// index vs. a Linux `SocketCAN` interface name vs. nothing at all for
    /// the mock backend), so there's no way to guess a working `selector`
    /// without first asking the backend what it sees.
    ListDevices {
        /// Backend to query. Only backends enabled at build time via
        /// Cargo features are available; see `--help` output of the
        /// binary you built for the enabled set, or run this with an
        /// invalid name to have it listed in the error.
        #[arg(short, long)]
        backend: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    match args.command.unwrap_or(Command::Run {
        config: "cand-relay.toml".into(),
    }) {
        Command::Run { config } => run(config).await,
        Command::ListDevices { backend } => list_devices(&backend),
    }
}

async fn run(config: PathBuf) -> anyhow::Result<()> {
    let config_str = std::fs::read_to_string(&config)
        .map_err(|e| anyhow::anyhow!("failed to read config file {}: {e}", config.display()))?;
    let config = BridgeConfig::from_toml_str(&config_str)?;

    let mut registry: BusRegistry = BusRegistry::new();
    for bus in &config.buses {
        let params = ChannelParams {
            bitrate: bus.bitrate,
            sample_point: bus.sample_point,
            selector: bus.selector.clone(),
            fd: bus.fd.as_ref().map(cand_relay_core::FdConfig::to_params),
        };
        if params.fd.is_some() && bus.backend != "vector" && bus.backend != "mock" {
            return Err(anyhow::anyhow!(
                "bus '{}': [bus.fd] (CAN FD) is currently only supported by the 'vector' backend",
                bus.name
            ));
        }
        let handle = build_bus_handle(&bus.backend, params)?;
        registry.insert(bus.name.clone(), handle);
        tracing::info!(
            "configured bus '{}' using backend '{}'",
            bus.name,
            bus.backend
        );

        if let Some(wakeup) = &bus.wakeup {
            spawn_wakeup(&bus.backend, wakeup)?;
        }
    }

    run_server(&config.server.listen, registry).await?;
    Ok(())
}

/// Starts a background thread that pulses a digital-output "wake-up" line
/// (see [`WakeupConfig`]) once at startup and, if `interval_secs` is
/// non-zero, repeatedly afterwards. Runs for the lifetime of the process --
/// there's nothing to join, since a wake-up pulse has no result the rest of
/// the bridge depends on; failures are only logged.
///
/// Uses a plain OS thread rather than a tokio task because the underlying
/// XL-API calls ([`cand_relay_backend_vector::WakeupPulser`]) are blocking
/// (including a real `std::thread::sleep` while the line is held high), so
/// running this directly on the async runtime would tie up a worker thread.
fn spawn_wakeup(
    backend: &str,
    #[cfg_attr(
        not(any(feature = "vector-windows", feature = "vector-linux")),
        allow(unused_variables)
    )]
    wakeup: &WakeupConfig,
) -> anyhow::Result<()> {
    match backend {
        "vector" => {
            #[cfg(any(feature = "vector-windows", feature = "vector-linux"))]
            {
                let selector = wakeup.selector.clone();
                let port_mask = wakeup.port_mask;
                let pulse_ms = wakeup.pulse_ms;
                let interval_secs = wakeup.interval_secs;
                std::thread::spawn(move || {
                    let pulser =
                        match cand_relay_backend_vector::WakeupPulser::open(&selector, port_mask) {
                            Ok(pulser) => pulser,
                            Err(e) => {
                                tracing::error!(
                                    "failed to open wake-up channel '{selector}' (port_mask \
                                     {port_mask:#04x}): {e}"
                                );
                                return;
                            }
                        };
                    loop {
                        match pulser.pulse(pulse_ms) {
                            Ok(()) => tracing::info!(
                                "pulsed wake-up line on '{selector}' (port_mask {port_mask:#04x}) \
                                 for {pulse_ms}ms"
                            ),
                            Err(e) => {
                                tracing::error!("failed to pulse wake-up line on '{selector}': {e}")
                            }
                        }
                        if interval_secs == 0 {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_secs(interval_secs));
                    }
                });
                Ok(())
            }
            #[cfg(not(any(feature = "vector-windows", feature = "vector-linux")))]
            {
                Err(anyhow::anyhow!(
                    "bus has a [wakeup] section but this build was compiled without the \
                     vector-windows/vector-linux feature"
                ))
            }
        }
        other => Err(anyhow::anyhow!(
            "wakeup config is not supported for backend '{other}' (only 'vector' currently)"
        )),
    }
}

/// Prints every device/channel the given backend can see, one per line,
/// each annotated with the `selector` value to use for it in
/// `cand-relay.toml`. The exact meaning of a "device" -- and how it
/// maps to `selector` -- is backend-specific:
///
/// - `vector`: one line per XL-API channel (as reported by
///   `xlGetDriverConfig`), independent of any Vector Hardware Manager
///   "application" setup. Only channels with `canCap=true` support classic
///   CAN; use that channel's `channelIndex` as `selector` (e.g.
///   `selector = "2"`).
/// - `socketcan`: one line per Linux CAN netdevice found under
///   `/sys/class/net` (real adapters and `vcan`/virtual interfaces alike).
///   `selector` is just the interface name (e.g. `selector = "can0"`); the
///   interface must already be brought up (`ip link set can0 up type can
///   bitrate <rate>`) for the bridge to open it.
/// - `mock`: no real devices; any `selector` value works.
fn list_devices(backend: &str) -> anyhow::Result<()> {
    let lines: Vec<String> = match backend {
        #[cfg(feature = "mock")]
        "mock" => cand_relay_backend_mock::list_devices(),
        #[cfg(feature = "socketcan")]
        "socketcan" => cand_relay_backend_socketcan::list_devices(),
        #[cfg(any(feature = "vector-windows", feature = "vector-linux"))]
        "vector" => cand_relay_backend_vector::list_channels(),
        other => {
            return Err(anyhow::anyhow!(
                "unknown or disabled backend '{other}'. This build was compiled with features: {}",
                enabled_backend_features().join(", ")
            ));
        }
    };
    for line in lines {
        println!("{line}");
    }
    Ok(())
}

/// Constructs a [`cand_relay_core::server::BusHandle`] for the given backend
/// name. Only backends enabled via Cargo feature at build time are
/// available; requesting a disabled/unknown backend is a startup-time
/// configuration error, not a compile-time one.
fn build_bus_handle(
    backend: &str,
    params: ChannelParams,
) -> anyhow::Result<cand_relay_core::server::BusHandle> {
    match backend {
        #[cfg(feature = "mock")]
        "mock" => {
            let (backend, _handle) = cand_relay_backend_mock::mock_backend();
            Ok(spawn_bus(backend, params))
        }
        #[cfg(feature = "socketcan")]
        "socketcan" => Ok(spawn_bus(
            cand_relay_backend_socketcan::SocketCanBackend::new(),
            params,
        )),
        #[cfg(any(feature = "vector-windows", feature = "vector-linux"))]
        "vector" => Ok(spawn_bus(
            cand_relay_backend_vector::VectorBackend::new(),
            params,
        )),
        other => Err(anyhow::anyhow!(
            "unknown or disabled backend '{other}'. This build was compiled with features: {}",
            enabled_backend_features().join(", ")
        )),
    }
}

fn enabled_backend_features() -> Vec<&'static str> {
    [
        cfg!(feature = "mock").then_some("mock"),
        cfg!(feature = "socketcan").then_some("socketcan"),
        cfg!(feature = "vector-windows").then_some("vector-windows"),
        cfg!(feature = "vector-linux").then_some("vector-linux"),
    ]
    .into_iter()
    .flatten()
    .collect()
}
