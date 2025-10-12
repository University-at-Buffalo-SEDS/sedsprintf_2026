# SEDSPRINTF_RS

A reimplementation of the `sedsprintf` library in Rust.

This library provides a safe and efficient way to handle telemetry data including serializing, routing, and converting
into strings for logging.

# Dependencies

- Rust
  get it from https://rustup.rs/
- Cmake
- A C++ compiler
- A C compiler
    - The thumbv7m-none-eabi target for Rust (if you want to build for ARM Cortex-M)
      Get it with:
  ```bash
  rustup target add thumbv7em-none-eabihf
  ```
  

- When using this library as a submodule in a different c or c++ project, make sure to add the following to your cmakelists.txt
  ```cmake
  add_subdirectory(${CMAKE_SOURCE_DIR}/sedsprintf_rs)
  target_link_libraries(${CMAKE_PROJECT_NAME} sedsprintf_rs)
  ```
  

- To add this repo as a subtree to allow for modifications, use the following command:
  ```bash
  git remote add sedsprintf-upstream https://github.com/Rylan-Meilutis/sedsprintf_rs.git
  git fetch sedsprintf-upstream
  git subtree add --prefix=sedsprintf_rs sedsprintf-upstream main
  
- To update the subtree, use the following command (Note all local changes must be committed before you can update):
  ```bash
  git subtree pull --prefix=sedsprintf_rs sedsprintf-upstream main -m "Merge sedsprintf_rs upstream main"
  ```
