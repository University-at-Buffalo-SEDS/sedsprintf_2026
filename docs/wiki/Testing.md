# Testing

This page explains what is validated in this repo and what `./build.py test` is expected to cover.

## Main entry point

Use:

```bash
./build.py test
```

In this repo, that command currently runs:

- strict clippy on the default host build
- strict clippy on the Python-feature build
- strict clippy on the embedded build
- `cargo test --features timesync`
- a short Criterion benchmark smoke pass
- `cargo build --features python`
- `cargo build --no-default-features --target thumbv7em-none-eabihf --features embedded`

That gives one command that validates host behavior, bindings, embedded compilation, and a light
performance smoke check.

## Nature of the tests

The test suite is split across several layers:

- Unit tests in `src/tests.rs`
- C ABI tests in `src/c_api.rs`
- Rust system tests in `tests/rust-system-test/`
- C system tests in `tests/c-system-test/`
- benchmark smoke in `benches/`

The unit tests cover packet conversion, queue behavior, dedupe, retry logic, router/relay routing,
discovery behavior, link-local routing boundaries, time sync internals, and reliability edge cases.

The system tests exercise full multi-node paths instead of isolated helper functions. They are the
main defense against behavioral regressions in routing, relay forwarding, reliable retransmit, and
time-sync coordination.

## Reliability coverage

Reliable transport is covered at multiple layers:

- unit tests verify ordered delivery, retransmit, ACK handling, and queue behavior
- router discovery tests verify that reliable packets fan out to all discovered candidate sides
- router discovery tests verify that pending end-to-end destinations are dropped when a discovered
  holder expires from topology
- relay discovery tests verify that learned end-to-end ACK holder state is pruned when discovery
  ages out the corresponding holder
- Rust system tests in `reliable_drop_test.rs` verify dropped-frame recovery over real router paths

For the current backport, the important contract is:

- per-link reliable delivery still handles loss, ordering, and CRC-triggered resend
- reliable serialized links also keep a packet pending until the currently discovered destination
  holders have all ACKed local delivery
- end-to-end ACK routing is directional rather than flooded
- disappeared holders are removed from the pending set so retries do not continue forever

## Discovery and routing coverage

Discovery-related tests cover:

- selective forwarding when a route is known
- fallback behavior when no route is known yet
- link-local-only endpoint containment
- advertised topology snapshots
- discovery plus time-sync interaction
- relay-side topology aggregation

These tests matter because the end-to-end reliable layer depends on discovery state to decide which
holders are still outstanding and where ACK traffic should go.

## System tests

Rust system tests currently cover:

- threaded and single-threaded router/relay stress paths
- reliable drop recovery
- end-to-end router/relay forwarding scenarios
- compression stability under load
- time-sync failover and multi-node coordination

C system tests currently cover:

- C ABI integration against the built library
- multi-board routing behavior
- time-sync helper integration
- board-topology time-sync scenarios

## Code coverage notes

This repo does not currently publish a line-coverage percentage in `build.py test`. Coverage here is
behavioral rather than percentage-based:

- unit tests cover internal state machines and edge cases
- system tests cover realistic end-to-end paths across Rust and C entry points
- build validation ensures Python and embedded targets still compile

If line-coverage reporting is needed later, it should be added as a separate explicit job so it
does not distort the normal development test path.
