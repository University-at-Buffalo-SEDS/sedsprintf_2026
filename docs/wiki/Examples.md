# Examples (Easy)

This page points to runnable examples and suggests a learning path.
For protocol details and role behavior, see [Time-Sync](Time-Sync).

## C/C++ example

- c-example-code/ ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/tree/main/c-example-code), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/tree/main/c-example-code))
- c-example-code/src/timesync_example.c ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/c-example-code/src/timesync_example.c), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/blob/main/c-example-code/src/timesync_example.c))

What it demonstrates:

- Building and linking the staticlib.
- Creating and sending packets.
- Receiving and dispatching to handlers.
- Time sync announce/request/response and offset math.

Suggested first steps:

1) Build the library with build.py ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/build.py), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/blob/main/build.py)) or CMake.
2) Compile the example and run it locally.
3) Watch the output to see packet creation and handling.

## Python example

- python-example/ ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/tree/main/python-example), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/tree/main/python-example))
- python-example/timesync_example.py ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/python-example/timesync_example.py), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/blob/main/python-example/timesync_example.py))

What it demonstrates:

- Installing the Python package.
- Logging packets and decoding values.
- Using the generated enums.
- Time sync announce/request/response and offset math.

Suggested first steps:

1) Build Python bindings with `build.py python` or `build.py maturin-install` (build.py: [GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/build.py), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/blob/main/build.py)).
2) Run the example script.
3) Inspect printed packets to see decoded values.

## Rust example (minimal)

If you want a minimal Rust example, start with [Usage-Rust](Usage-Rust) and build a small router with one endpoint
handler. For a runnable example, see:

- rust-example-code/timesync_example.rs ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/rust-example-code/timesync_example.rs), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/blob/main/rust-example-code/timesync_example.rs))
- rust-example-code/relay_example.rs ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/rust-example-code/relay_example.rs), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/blob/main/rust-example-code/relay_example.rs))
- rust-example-code/reliable_example.rs ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/rust-example-code/reliable_example.rs), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/blob/main/rust-example-code/reliable_example.rs))
- rust-example-code/queue_timeout_example.rs ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/rust-example-code/queue_timeout_example.rs), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/blob/main/rust-example-code/queue_timeout_example.rs))
- rust-example-code/multinode_sim_example.rs ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/rust-example-code/multinode_sim_example.rs), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/blob/main/rust-example-code/multinode_sim_example.rs))

## RTOS time sync examples

- rtos-example-code/freertos_timesync.c ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/rtos-example-code/freertos_timesync.c), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/blob/main/rtos-example-code/freertos_timesync.c))
- rtos-example-code/threadx_timesync.c ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/rtos-example-code/threadx_timesync.c), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/blob/main/rtos-example-code/threadx_timesync.c))

Recommended structure:

- Define one `EndpointHandler` for a single `DataEndpoint`.
- Create a router in sink mode.
- Call `log_*` with a typed payload.
- Call `rx_serialized` with the bytes you just sent (loopback).

## Recommended path

1) Read [Overview](Overview)
2) Read [Concepts](Concepts)
3) Try one example in your target language
4) Read [technical/Architecture](technical/Architecture) for the implementation details
