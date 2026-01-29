# SEDSPRINTF_RS

An implementation of the `sedsprintf` telemetry protocol in Rust.

---

## Authors

- [@Rylan-Meilutis](https://github.com/rylan-meilutis) (Original Author, Maintainer, and co-creator of the protocol)
- [@origami-yoda](https://github.com/origami-yoda) (Co-creator of the protocol and co-author of the original C++
  implementation)

---

## About

This library started out as a rewrite of the original sedsprintf C++ library
found [here](https://github.com/University-at-Buffalo-SEDS/sedsprintf).

After the initial rewrite, many improvements were made to the rust implementation including better safety, easier
extension, and improved performance.
This caused the C++ implementation to be rewritten to keep feature parity with the rust version.
After about of month of this, we decided that we were no longer going to use the C++ version, and thus the project was
archived and is no longer being maintained.
With the Rust version being the sole implementation, we have continued to improve it and add new features like python
bindings, packet compression, and a bitmap for endpoints to further reduce packet size.
This library is now being used in multiple projects including embedded code on the rocket and on the rust based ground
station. Sedsprintf_rs is now capable of acting as a new network, passing telemetry data to endpoints across hardware
and software networks (uart, can ethernet, etc.) and across differing platforms and protocols (tcp, udp, etc.).

---

## Overview

This library provides a safe and efficient way to handle telemetry data including serializing, routing, and converting
into strings for logging. The purpose of this library is to provide an easy and consistent way to create and transport
any data to any destination without needing to implement the core routing on every platform, the library handles the
creation and routing of the data while allowing for easy extension to support new data types and destinations. The user
of the library is responsible for implementing 2 core functions and one function per local data endpoint.
The core functions are as follows:

- A function to send raw bytes to the all other nodes (e.g. UART, SPI, I2C, CAN, etc.)
- A function to receive raw bytes from all other nodes and passes the packet to the router
  (e.g. UART, SPI, I2C, CAN, etc.)
- A function to handle local data endpoints (e.g. logging to console, writing to file, sending over radio, etc.)
  (Note: each local endpoint needs its own function)

Sedsprintf_rs also provides helpers to convert the telemetry data into strings for logging purposes.
The library also handles the serialization and deserialization of the telemetry data.

Sedsprintf_rs is platform-agnostic and can be used on any platform that supports Rust. The library is primarily designed
to be used in embedded systems and used by a C program, but can also be used in desktop applications and other rust
codebases.

Sedsprintf_rs also supports python bindings via pyo3. to use you need maturin installed to build the python package.

The size of the header in a serialized packet is around 20 bytes (the size will change based on the total number of
endpoints in your system and the length of the sender string), plus a 4-byte CRC32 trailer. As a rough example, a packet
containing three floats is on the order of mid-30s bytes total. This small size makes it ideal for use in low bandwidth
environments.

---

## Version 3.0.0 highlights

- Router side tracking is now internal. You generally call `rx_serialized(...)` / `rx(...)` without threading a side ID
  through your own handlers. Use the `*_from_side` variants only when you must override ingress (e.g. custom relays or
  multi-link bridges).
- TCP-like reliability is available for schema types marked `reliable` / `reliable_mode`, including ACKs, retransmits,
  and optional ordering. Enable it per side (`RouterSideOptions`) and disable it when your transport is already
  reliable.
- All serialized frames include a CRC32 trailer for integrity checks. Corrupt frames are dropped; reliable modes trigger
  retransmit requests.
- Full changelog: [v2.4.0...v3.0.0](https://github.com/Rylan-Meilutis/sedsprintf_rs/compare/v2.4.0...v3.0.0)

---

## Building

To build the library in a C project, just include the library as a submodule or subtree and link it in your
cmakelists.txt as shown below.
For other build systems, you can build the library as a static or dynamic library using cargo and link it to your
project.

Building with python bindings can be done with the build script on posix systems:

```
./build.py release maturin-develop
```

When building in an embedded environment the library will compile to a static library that can be linked to your C code.
this library takes up about 100kb of flash and does require heap allocation to be available through either freertos, or
by creating shims that expose pvPortMalloc and vPortFree.

### build.py usage

```
./build.py [OPTIONS]

Options:
  release                 Build in release mode.
  test                    Run cargo tests (also validates embedded+python builds).
  embedded                Build for the embedded target (enables embedded feature).
  python                  Build with Python bindings (enables python feature).
  maturin-build           Run maturin build with the .pyi .gitignore hack.
  maturin-develop         Run maturin develop with the .pyi .gitignore hack.
  maturin-install         Build wheel and install it with uv pip install.
  target=<triple>         Set Rust compilation target (e.g. target=thumbv7em-none-eabihf).
  device_id=<id>          Set DEVICE_IDENTIFIER env var for the build.
  max_stack_payload=<n>   Set MAX_STACK_PAYLOAD for define_stack_payload!(env="MAX_STACK_PAYLOAD", ...).
  env:KEY=VALUE           Set arbitrary environment variable(s) for the build (repeatable).
```

Examples:

```
./build.py release
./build.py embedded release target=thumbv7em-none-eabihf device_id=FC
./build.py python
./build.py maturin-install env:MAX_RECENT_RX_IDS=256 env:MAX_STACK_PAYLOAD=128
```

## Dependencies

- Rust → https://rustup.rs/
- CMake
- A C++ compiler
- A C compiler

## Usage

### Linking from a C/C++ CMake project

```
# Example: building for an embedded target
set(SEDSPRINTF_RS_TARGET "thumbv7m-none-eabi" CACHE STRING "" FORCE)
set(SEDSPRINTF_EMBEDDED_BUILD ON CACHE BOOL "" FORCE)

# set the sender name
set(SEDSPRINTF_RS_DEVICE_IDENTIFIER "FC26_MAIN" CACHE STRING "" FORCE)

# optional compile-time env overrides
set(SEDSPRINTF_RS_MAX_STACK_PAYLOAD "256" CACHE STRING "" FORCE)
set(SEDSPRINTF_RS_ENV_MAX_QUEUE_SIZE "65536" CACHE STRING "" FORCE)

# Use the provided CMake glue
add_subdirectory(${CMAKE_SOURCE_DIR}/sedsprintf_rs sedsprintf_rs_build)

# Link against the imported target
target_link_libraries(${CMAKE_PROJECT_NAME} PRIVATE sedsprintf_rs::sedsprintf_rs)
```

- Configure telemetry schema via `telemetry_config.json` (endpoints + message types). The Rust enum metadata is
  generated
  from this JSON by `define_telemetry_schema!` in `src/config.rs`.
  NOTE: (ON EVERY SYSTEM THIS LIBRARY IS USED, THE CONFIG ENUMS MUST BE THE SAME OR UNDEFINED BEHAVIOR MAY OCCUR). So
  for
  most applications I would recommend making a fork and setting the config values you need for your application.

---

## Setting the device / platform name

Each build of `sedsprintf_rs` embeds a **device identifier** which appears in every telemetry packet header.

Rust resolves it using:

```
pub const DEVICE_IDENTIFIER: &str = match option_env!("DEVICE_IDENTIFIER") {
    Some(v) => v,
    None => "TEST_PLATFORM",
};
```

### Set it globally using `.cargo/config.toml` (recommended)

Create:

```
# .cargo/config.toml
[env]
DEVICE_IDENTIFIER = "GROUND_STATION_26"
```

After this, any `cargo build`, `cargo run`, or CI build will embed `"GROUND_STATION_26"` automatically.

No build script changes required.

---

### Setting the name from CMake

```
set(SEDSPRINTF_RS_DEVICE_IDENTIFIER "FC26_MAIN" CACHE STRING "" FORCE)
```

Note: This must be set **before** including the sedsprintf_rs CMake as a subdirectory.

Typical examples:

```cmake
# Flight computer firmware
set(SEDSPRINTF_RS_DEVICE_IDENTIFIER "FC26_MAIN" CACHE STRING "" FORCE)
set(SEDSPRINTF_RS_TARGET "thumbv7em-none-eabihf" CACHE STRING "" FORCE)
set(SEDSPRINTF_EMBEDDED_BUILD ON CACHE BOOL "" FORCE)
set(SEDSPRINTF_RS_MAX_STACK_PAYLOAD "256" CACHE STRING "" FORCE)
set(SEDSPRINTF_RS_ENV_MAX_QUEUE_SIZE "65536" CACHE STRING "" FORCE)

# or

# Ground station app
set(SEDSPRINTF_RS_DEVICE_IDENTIFIER "GS26" CACHE STRING "" FORCE)
set(SEDSPRINTF_RS_TARGET "" CACHE STRING "" FORCE)
set(SEDSPRINTF_EMBEDDED_BUILD OFF CACHE BOOL "" FORCE)
set(SEDSPRINTF_RS_ENV_MAX_QUEUE_SIZE "65536" CACHE STRING "" FORCE)
```

### Manually via build.py

```bash
# Host build
./build.py release device_id=GROUND_STATION
# Embedded build
./build.py embedded release target=thumbv7em-none-eabihf device_id=FC
```

---

## Telemetry config (JSON + GUI editor)

The telemetry schema lives in `telemetry_config.json` and drives the generated `DataEndpoint` and `DataType` enums.
You can edit it directly or use the GUI editor:

```bash
./telemetry_config_editor.py
```

The editor auto-discovers the JSON path from `src/config.rs` (or `SEDSPRINTF_RS_SCHEMA_PATH`), lets you add
endpoints/types, and writes the schema back to `telemetry_config.json`.

Note: `TelemetryError` (data type and endpoint) is built-in and must not appear in the JSON schema.

Note: The editor uses Tkinter. On some Linux distros you may need to install it
(e.g. `sudo apt install python3-tk`).

Example `telemetry_config.json`:

```json
{
  "endpoints": [
    { "rust": "Radio", "name": "RADIO", "doc": "Downlink radio", "broadcast_mode": "Default" },
    { "rust": "SdCard", "name": "SD_CARD", "doc": "Onboard logging", "broadcast_mode": "Default" }
  ],
  "types": [
    {
      "rust": "GpsData",
      "name": "GPS_DATA",
      "doc": "Lat/Lon/Alt",
      "class": "Data",
      "element": { "kind": "Static", "data_type": "Float32", "count": 3 },
      "endpoints": ["Radio", "SdCard"]
    }
  ]
}
```

---

## Example CMakeLists.txt

```cmake
cmake_minimum_required(VERSION 3.22)
project(my_app C CXX)

add_executable(my_app
    src/main.c
)

# ---- sedsprintf_rs configuration ----
set(SEDSPRINTF_RS_DEVICE_IDENTIFIER "FC26_MAIN" CACHE STRING "" FORCE)
set(SEDSPRINTF_RS_TARGET "thumbv7em-none-eabihf" CACHE STRING "" FORCE)
set(SEDSPRINTF_EMBEDDED_BUILD ON CACHE BOOL "" FORCE)
set(SEDSPRINTF_RS_MAX_STACK_PAYLOAD "256" CACHE STRING "" FORCE)
set(SEDSPRINTF_RS_ENV_MAX_QUEUE_SIZE "65536" CACHE STRING "" FORCE)

# Add the submodule/subtree root (adjust path as needed)
add_subdirectory(${CMAKE_SOURCE_DIR}/sedsprintf_rs sedsprintf_rs_build)

target_link_libraries(my_app PRIVATE sedsprintf_rs::sedsprintf_rs)
```

---

## Using this repo as a subtree

```
git remote add sedsprintf-upstream https://github.com/University-at-Buffalo-SEDS/sedsprintf_2026.git
git fetch sedsprintf-upstream

git config subtree.sedsprintf_rs.remote sedsprintf-upstream
git config subtree.sedsprintf_rs.branch main

git subtree add --prefix=sedsprintf_rs sedsprintf-upstream main
```

To Switch branches:

```bash
git config subtree.sedsprintf_rs.branch <the-new-branch>
```

Update:

```bash
git subtree pull --prefix=sedsprintf_rs sedsprintf-upstream main \
    -m "Merge sedsprintf_rs upstream main"
```

Helper scripts:

```bash
./scripts/subtree_update_no_stash.py
./scripts/subtree_update.py            # stash → update → stash-pop
```

---

## Using this repo as a submodule

If you prefer a **submodule** instead of a subtree:

```bash
git submodule add -b main https://github.com/University-at-Buffalo-SEDS/sedsprintf_2026.git sedsprintf_rs

git config submodule.sedsprintf_rs.branch main   # (or dev, etc.)
```

Initialize:

```bash
git submodule update --init --recursive
```

Update using helper scripts:

The scripts:

- read `submodule.sedsprintf_rs.branch`
- fetch `origin/<branch>`
- fast-forward the submodule repo
- stage & commit the updated submodule pointer in the parent repo

---

## Embedded allocator hook example (C)

```C
// telemetry_hooks.c
#include <stddef.h>
#include <stdlib.h>

/*
 * Rust expects these functions to exist for heap allocations:
 *
 *   void *telemetryMalloc(size_t);
 *   void telemetryFree(void *);
 *   void seds_error_msg(const char *, const size_t);
 *
 */

void *telemetryMalloc(size_t xSize)
{
     return malloc(xSize);
}

void telemetryFree(void *pv)
{
    free(pv);
}

void seds_error_msg(const char *str, const size_t len)
{
    // Implement your logging mechanism here, for example:
    fwrite(str, 1, len, stderr);
    fwrite("\n", 1, 1, stderr);
}
```
