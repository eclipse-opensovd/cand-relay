<!--
SPDX-FileCopyrightText: 2026 Copyright (c) Contributors to cand-relay

See the NOTICE file(s) distributed with this work for additional
information regarding copyright ownership.

This program and the accompanying materials are made available under the
terms of the Apache License Version 2.0 which is available at
https://www.apache.org/licenses/LICENSE-2.0

SPDX-License-Identifier: Apache-2.0
-->

# cand-relay

A `socketcand`-compatible CAN relay. It exposes CAN interfaces over TCP
using the socketcand wire protocol. Each kind of hardware is supported by a
backend, and each backend is enabled with a Cargo feature.

It was built so the
[Classic Diagnostic Adapter](https://github.com/eclipse-opensovd/classic-diagnostic-adapter)
(CDA) can use CAN hardware that isn't a Linux SocketCAN device, mainly
Vector's XL-API interfaces (VN15xx, VN16xx, VN56xx, VN76xx, ...). CDA needs no
changes: it connects through its existing `can-socketcand` transport.

> **Security:** cand-relay has no authentication or encryption. Anyone who
> can reach the listen port can read traffic from the CAN bus and send
> frames onto it. Bind to `127.0.0.1` or a trusted, isolated network only.

## Quick start

```sh
cargo build --release -p cand-relay --features socketcan   # or vector-windows, see below
cand-relay list-devices --backend socketcan                # find your selector
cp cand-relay.toml.example cand-relay.toml                 # set backend/selector/bitrate
cand-relay run --config cand-relay.toml                    # `cand-relay` alone runs ./cand-relay.toml
```

Then point CDA at `socketcand:<relay-host>:29536:<bus-name>`
(see [Running against CDA](#running-against-cda)).

## Why

CDA's CAN transport (`cda-comm-can`) supports two targets:

- a local Linux SocketCAN interface, or
- a remote `socketcand` daemon over TCP.

Vector interfaces are neither. They are accessed through Vector's XL Driver
Library (XL-API). cand-relay fills the gap by acting as a `socketcand` server
in front of an XL-API channel, or any other backend.

Only the raw-frame subset of the protocol is implemented (see
[Wire protocol](#wire-protocol)). ISO-TP segmentation happens on the client
side in CDA, so the relay only forwards frames. CAN FD bus timing is
configured on the relay side (`[bus.fd]`); FD frames are exchanged with the
`< fdframe >` / `< fdsend >` raw mode extension from
[socketcand PR #79](https://github.com/linux-can/socketcand/pull/79).

## Architecture

- `cand-relay-core`: socketcand protocol and async TCP server. It only
  depends on the `CanBackend` trait, not on any specific backend.
- `cand-relay-backend-*`: one crate per CAN hardware or driver. Adding
  hardware never requires changes to `cand-relay-core`.
  - `cand-relay-backend-vector`: Vector XL-API (requires the vendor SDK at
    build time, see [docs/vector-sdk.md](docs/vector-sdk.md)).
  - `cand-relay-backend-socketcan`: Linux SocketCAN passthrough, e.g. to share
    a Linux-hosted adapter over the network, or for development.
  - `cand-relay-backend-mock`: in-memory backend for tests.
- `cand-relay`: the binary. It reads a TOML config and runs the server with
  the backends that were compiled in.

See [docs/architecture.md](docs/architecture.md) for the `CanBackend` trait
contract and how to add a backend.

## Building

| Cargo feature    | Backend                | Notes                                     |
|------------------|------------------------|-------------------------------------------|
| `mock` (default) | in-memory mock         | no hardware access, for tests only        |
| `socketcan`      | Linux SocketCAN        | Linux only                                |
| `vector-windows` | Vector XL-API, Windows | needs XL Driver Library SDK + LLVM        |
| `vector-linux`   | Vector XL-API, Linux   | needs Vector Linux XL-API package + LLVM  |

```sh
# Default: mock backend only. This binary cannot talk to real hardware.
cargo build --release -p cand-relay

# Linux SocketCAN
cargo build --release -p cand-relay --features socketcan

# Vector XL-API on Windows
VXLAPI_H="C:\...\include\vxlapi.h" VXLAPI_LIB_DIR="C:\...\lib" \
  cargo build --release -p cand-relay --features vector-windows --target x86_64-pc-windows-msvc
```

The Vector backend is only compiled when `vector-windows` or `vector-linux` is
enabled. Building it requires Vector's XL Driver Library, which is not
bundled and can't be redistributed, and LLVM/libclang for `bindgen`. See
[docs/vector-sdk.md](docs/vector-sdk.md) for setup.

### Binary releases

Prebuilt binaries are only published **without** the Vector backend. A
Vector-enabled binary contains material derived from Vector's SDK, so you
have to build it yourself. See
[docs/vector-sdk.md](docs/vector-sdk.md#distributing-binaries).

## Configuration

Start from `cand-relay.toml.example`. The relay reads `cand-relay.toml` by
default (override with `--config`) and listens on `0.0.0.0:29536` in the
example (`[server] listen`). Each `[[bus]]` entry maps a bus name to a backend
and a backend-specific `selector`. Clients open a bus with `< open <bus> >`,
which matches the bus name in CDA's `socketcand:<host>:<port>:<bus>` address.

Logging uses `tracing`; set the level with `RUST_LOG`, e.g.
`RUST_LOG=info cand-relay` or `RUST_LOG=cand_relay_core=trace`.

### Finding `selector` values: `list-devices`

`selector` means something different for each backend: a Vector XL-API
channel, a Linux interface name, or nothing at all for mock. To find valid
values, ask the backend what it sees. Only backends compiled into your build
can be queried:

```sh
cand-relay list-devices --backend vector
# xlGetDriverConfig status=0 channelCount=7
# [0] name="VN5610A Channel 1" hwType=... channelIndex=0 isOnBus=0 busCaps=0x... canCap=false fdCap=false -> selector="0"
# [2] name="VN5610A Channel 3" hwType=... channelIndex=2 isOnBus=0 busCaps=0x... canCap=true fdCap=true -> selector="2"
```

Each line ends with the `selector` value to use:

- **`vector`**: one line per XL-API channel. Nothing needs to be set up in
  Vector Hardware Manager first. Only channels with `canCap=true` support
  CAN; the others may be LIN, I/O or DAIO channels. As `selector`, use either
  the `channelIndex` (e.g. `"2"`) or the exact `name` (e.g.
  `"VN5610A Channel 3"`). The index can change when devices are added or
  removed; the name stays stable as long as the model/channel combination is
  unique.
- **`socketcan`** (Linux): one line per CAN interface under `/sys/class/net`,
  including `vcan`. The selector is the interface name (e.g. `"can0"`).
  `list-devices` doesn't configure anything, so bring the interface up first:
  `sudo ip link set can0 up type can bitrate 500000`, or for a virtual bus
  `sudo ip link add dev vcan0 type vcan && sudo ip link set up vcan0`.
- **`mock`**: no real devices; any `selector` is accepted.

### Waking up a sleeping ECU: `[bus.wakeup]`

Some ECUs default to a low-power sleep state and won't respond on the CAN
bus at all until woken by a dedicated physical signal. Vector devices like
the VN5610A/VN5620 expose this as their own DAIO channel -- Vector calls it
the "Activation Line"/"WakeUp Line" (`XL_TRANSCEIVER_TYPE_DAIO_AL_WU` in
`vxlapi.h`) -- separate from the actual CAN channel; look for it in
`list-devices --backend vector` output (`canCap=false`, a DAIO-capable
channel).

Add a `[bus.wakeup]` table under the relevant `[[bus]]` entry to pulse a
digital output line on that channel once at startup, and optionally on a
repeating interval (e.g. for an ECU that goes back to sleep):

```toml
[[bus]]
name = "can0"
backend = "vector"
bitrate = 500000
selector = "3"

[bus.wakeup]
selector = "VN5610A Channel 5"
port_mask = 1       # XL_DAIO_PORT_MASK_DIGITAL_D0; check your device's manual
pulse_ms = 300      # how long to hold the line high
interval_secs = 5   # 0 (default) = pulse once at startup only
```

Currently only supported for the `vector` backend; a bus with a `[wakeup]`
section on any other backend fails at startup with a clear error rather
than silently doing nothing.

### CAN FD: `[bus.fd]`

For CAN FD buses (`vector` backend; also accepted by `mock` for testing.
Check `fdCap=true` in `list-devices`), add an `[bus.fd]` table. The arbitration phase uses
`bitrate`/`sample_point`, the data phase the values below:

```toml
[[bus]]
name = "can0"
backend = "vector"
bitrate = 500000
sample_point = 80
selector = "VN7610 Channel 2"

[bus.fd]
data_bitrate = 1000000
data_sample_point = 75
iso = true           # default; false = non-ISO (Bosch) CAN FD
tx_mode = "fd_brs"   # "fd_brs" (default) | "fd" | "classic"
```

The bridge computes the bit timing for the 80 MHz CAN FD controller clock
and opens the channel with XL-API interface version 4. Received FD frames
are relayed as `< fdframe >` messages (with BRS/ESI flags, up to 64 data
bytes); classic frames keep using `< frame >`. A client `< fdsend >` is put on
the bus as CAN FD with exactly the requested BRS flag, while a classic
`< send >` is transmitted per `tx_mode` (frames longer than 8 bytes are
always FD). `< fdsend >` is ignored on buses without `[bus.fd]`. Note that an
ECU on an FD bus may answer with FD single frames longer than 8 bytes
(ISO 15765-2:2016 `SF_DL` escape, e.g. `00 14 62 F1 90 ...`), which the
client's ISO-TP layer must understand.

Bus parameters can only be applied if the bridge gets init access to the
channel -- close other tools using it (DTS Monaco, CANoe, ...) first.

## Running against CDA

Point CDA's `[can]` config at the relay:

```toml
[[can.transport_overrides]]
# ecu = "..."
address = "socketcand:<relay-host>:29536:can0"
```

Then start the relay (`run` is the default subcommand):

```sh
cand-relay run --config cand-relay.toml
```

## Wire protocol

Messages are plain ASCII enclosed in `<` / `>`, with flexible whitespace and
no length prefix. Implemented subset:

```text
server: < hi >
client: < open <bus> >
server: < ok >                          (closes the connection if the bus is unknown)
client: < rawmode >                     (no reply)

server: < frame <id_hex> <sec.usec> <hexdata> >             received classic frame
client: < send <id_hex> <dlc> <b0> <b1> ... >               transmit frame (up to 64 bytes)
server: < fdframe <id_hex> <flags> <sec.usec> <HEXDATA> >   received CAN FD frame
client: < fdsend <id_hex> <flags> <len> <b0> <b1> ... >     transmit CAN FD frame
```

`fdframe` / `fdsend` follow the socketcand CAN FD raw mode extension
([PR #79](https://github.com/linux-can/socketcand/pull/79)): `flags` is a hex
value of `CANFD_BRS` (0x01) and `CANFD_ESI` (0x02), `len` is the decimal
payload length (0..64), and payloads that are not a valid CAN FD length are
zero-padded to the next valid one (e.g. 9 -> 12). Example:
`< fdsend 123 01 12 11 22 33 44 55 66 77 88 99 aa bb cc >`.

Other socketcand modes (BCM, ISO-TP, control) and unknown messages are
silently ignored. CAN IDs with more than 3 hex digits are treated as
extended (29-bit).

## Development

The `Makefile` pins the toolchains (stable for build/test, a dated nightly
for `rustfmt`/Clippy) and wraps all checks CI runs; see `make help`.

```sh
make setup-devenv            # install toolchains, cargo-deny, uv and the pre-commit hooks
make build                   # default features (mock backend)
make test-all-features       # all backends that build without vendor SDKs (mock, socketcan)
make fmt                     # format
make lint-nightly            # Clippy with the shared OpenSOVD lint set
make deny                    # cargo-deny: licenses, sources, bans, advisories
make precommit-all-features  # everything CI runs
```

The Vector backend features (`vector-windows`, `vector-linux`) need the
non-redistributable XL Driver Library, so they're excluded from the
`*-all-features` targets and from CI.
Builds with the SDK installed locally use separate targets:

```sh
make build-vector-windows VXLAPI_H=".../include/vxlapi.h" VXLAPI_LIB_DIR=".../lib"
make lint-vector-linux    VXLAPI_H=".../vxlapi.h" VXLAPI_LIB_DIR=".../lib"
```

## Testing

```sh
make test
```

The `cand-relay-core` integration test runs the real server against the mock
backend with a raw TCP client. It checks the exact protocol subset that
CDA's client (`tokio-socketcan-isotp`) relies on, so no hardware is needed.

## License

Apache-2.0, see `LICENSES/Apache-2.0.txt` and `NOTICE`.

"Vector", "XL Driver Library" and "XL-API" are trademarks of Vector
Informatik GmbH, used here only to identify the hardware/API this project
interoperates with. This project is not affiliated with or endorsed by
Vector Informatik GmbH.
