# SEDSPRINTF_RS

A reimplementation of the `sedsprintf` library in Rust.

This library provides a safe and efficient way to handle telemetry data including serializing, routing, and converting
into strings for logging.

# Dependencies

- Rust
  get it from https://rustup.rs/
- Cmake
- A C++ compiler
    - The thumbv7m-none-eabi target for Rust (if you want to build for ARM Cortex-M)
      Get it with:
  ```bash
  rustup target add thumbv7em-none-eabihf
  ```