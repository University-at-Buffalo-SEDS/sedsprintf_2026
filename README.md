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
endpoints in your system and the length of the sender string), so a packet containing three floats is 32 bytes total.
This small size makes it ideal for use in low bandwidth environments.

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

# set the sender name
set(SEDSPRINTF_RS_DEVICE_IDENTIFIER "FC26_MAIN" CACHE STRING "" FORCE)

# Use the provided CMake glue
add_subdirectory(${CMAKE_SOURCE_DIR}/sedsprintf_rs/cmake sedsprintf_rs_build)

# Link against the imported target
target_link_libraries(${CMAKE_PROJECT_NAME} PRIVATE sedsprintf_rs::sedsprintf_rs)
```

- Set up the config.rs to match your application needs. All config options are in the config.rs file and are very
  self-explanatory.
  NOTE: (ON EVERY SYSTEM THIS LIBRARY IS USED, THE CONFIG ENUMS MUST BE THE SAME OR UNDEFINED BEHAVIOR MAY OCCUR). So
  for most
  applications I would recommend making a fork and setting the config values you need for your application.

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

# or

# Ground station app
set(SEDSPRINTF_RS_DEVICE_IDENTIFIER "GS26" CACHE STRING "" FORCE)
```

### Manually via build.py

```bash
# Host build
./build.py release device_id=GROUND_STATION
# Embedded build
./build.py embedded release target=thumbv7em-none-eabihf device_id=FC
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
