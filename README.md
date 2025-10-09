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
  

- WHen using this library as a submodule in a different c or c++ project, make sure to add the following to your cmakelists.txt
  ```cmake
  add_subdirectory(${CMAKE_SOURCE_DIR}/sedsprintf_rs)
  target_link_libraries(${CMAKE_PROJECT_NAME} sedsprintf_rs)
  ```