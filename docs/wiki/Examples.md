# Examples (Easy)

This page points to runnable examples and suggests a learning path.

## C/C++ example

- `c-example-code/`
- `c-example-code/src/timesync_example.c`

What it demonstrates:

- Building and linking the staticlib.
- Creating and sending packets.
- Receiving and dispatching to handlers.
- Time sync announce/request/response and offset math.

Suggested first steps:

1) Build the library with `build.py` or CMake.
2) Compile the example and run it locally.
3) Watch the output to see packet creation and handling.

## Python example

- `python-example/`
- `python-example/timesync_example.py`

What it demonstrates:

- Installing the Python package.
- Logging packets and decoding values.
- Using the generated enums.
- Time sync announce/request/response and offset math.

Suggested first steps:

1) Build Python bindings with `build.py python` or `build.py maturin-install`.
2) Run the example script.
3) Inspect printed packets to see decoded values.

## Rust example (minimal)

If you want a minimal Rust example, start with `docs/wiki/Usage-Rust.md` and build a small router with one endpoint
handler. For a runnable example, see:

- `rust-example-code/timesync_example.rs`
- `rust-example-code/relay_example.rs`
- `rust-example-code/reliable_example.rs`
- `rust-example-code/queue_timeout_example.rs`
- `rust-example-code/multinode_sim_example.rs`

## RTOS time sync examples

- `rtos-example-code/freertos_timesync.c`
- `rtos-example-code/threadx_timesync.c`

Recommended structure:

- Define one `EndpointHandler` for a single `DataEndpoint`.
- Create a router in sink mode.
- Call `log_*` with a typed payload.
- Call `rx_serialized` with the bytes you just sent (loopback).

## Recommended path

1) Read docs/wiki/Overview.md
2) Read docs/wiki/Concepts.md
3) Try one example in your target language
4) Read docs/wiki/technical/Architecture.md for the implementation details
