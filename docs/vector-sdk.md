<!--
SPDX-FileCopyrightText: 2026 Copyright (c) Contributors to cand-relay

See the NOTICE file(s) distributed with this work for additional
information regarding copyright ownership.

This program and the accompanying materials are made available under the
terms of the Apache License Version 2.0 which is available at
https://www.apache.org/licenses/LICENSE-2.0

SPDX-License-Identifier: Apache-2.0
-->

# Vector XL Driver Library SDK

The `windows`/`linux` features of `cand-relay-backend-vector` require Vector's
XL Driver Library: `vxlapi.h`, plus `vxlapi64.dll` and its import library on
Windows, or `libxlapi.so` on Linux.

## Why it isn't bundled or fetched automatically

- Vector's license for the XL Driver Library states: *"The DLLs (dynamic
  linked libraries) of the XL Driver Library may not be distributed solely or
  as part of any programs without a previous written individual agreement
  with Vector."* Bundling the library, fetching it in CI or downloading it on
  first run would all be redistribution.
- The [download page](https://www.vector.com/int/en/download/xl-driver-library/)
  is behind a form that emails you a link, so there is no stable URL to
  script against, and `cdn.vector.com` blocks bots.

In practice anyone running a Vector interface already has the driver
package installed, which includes the XL Driver Library.

## Building against a local install

Install LLVM first (e.g. `winget install LLVM.LLVM`): `bindgen` needs
`libclang.dll` on `PATH` or via `LIBCLANG_PATH` at build time. The resulting
binary does not depend on LLVM at runtime.

```sh
VXLAPI_H="C:\Users\<you>\...\XL Driver Library\include\vxlapi.h" \
VXLAPI_LIB_DIR="C:\Users\<you>\...\XL Driver Library\lib" \
  cargo build --release -p cand-relay --features vector-windows --target x86_64-pc-windows-msvc
```

If `VXLAPI_H` is unset, the build falls back to
`C:\Users\Public\Documents\Vector\XL Driver Library 25.20.14.0\bin\vxlapi.h`
and fails with an error if that doesn't exist either.

At runtime the binary dynamically loads `vxlapi64.dll` / `libxlapi.so` from
`PATH` or next to the executable. The Vector driver install provides it.

For `vector-linux`, point `VXLAPI_H` / `VXLAPI_LIB_DIR` at Vector's Linux
XL-API package (`vxlapi.h`, `libxlapi.so`) and build with
`--features vector-linux`.

## Distributing binaries

Only builds **without** the Vector backend (`mock`, `socketcan`) are
published. A Vector-enabled binary contains material derived from the SDK
even without the DLL:

- FFI struct layouts and constants generated from `vxlapi.h`, and
- on Windows, import stubs statically linked from `vxlapi64.lib`.

Whether Vector's license allows distributing such a binary has not been
clarified, so don't publish one. Shipping it together with the DLL
definitely requires a written agreement with Vector (`support@vector.com`).
