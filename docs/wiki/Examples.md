# Examples (Easy)

This page points to runnable examples and suggests a learning path.

## C/C++ example
- `c-example-code/`

What it demonstrates:
- Building and linking the staticlib.
- Creating and sending packets.
- Receiving and dispatching to handlers.

Suggested first steps:
1) Build the library with `build.py` or CMake.
2) Compile the example and run it locally.
3) Watch the output to see packet creation and handling.

## Python example
- `python-example/`

What it demonstrates:
- Installing the Python package.
- Logging packets and decoding values.
- Using the generated enums.

Suggested first steps:
1) Build Python bindings with `build.py python` or `build.py maturin-install`.
2) Run the example script.
3) Inspect printed packets to see decoded values.

## Rust example (minimal)
If you want a minimal Rust example, start with `docs/wiki/Usage-Rust.md` and build a small router with one endpoint handler.

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
