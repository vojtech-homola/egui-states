# Tests

## Folder map

Folder names describe their contents; Cargo package names are separate and remain
unchanged, so existing `cargo test -p ...` commands still work.

| Folder | Purpose |
| --- | --- |
| `python/` | Python library tests, discovered by pytest. |
| `python_test_helpers/` | Ordinary Python modules for shared port reservations and bounded waits. Used by library and example tests. |
| `rust-integration/` | Cargo integration and compiler tests, shared client scenarios, and executable probes used by Python tests. Package: `egui_states_test_support`. |
| `shared-test-states/` | Handwritten state and value definitions used to generate bindings for both Rust and Python integration tests. Package: `egui_states_test_schema`. |
| `generated-code-consumer/` | A small handwritten Rust project that compiles generated bindings, implements a custom derive, holds awkward but valid inputs (`awkward.rs`), and contains intentionally invalid macro inputs. Used by the Rust compiler tests and Python binding tests. |
| `rust-process-helpers/` | Dependency-free test crate for subprocess deadlines and captured output. Package: `egui_states_test_process`; a dev-dependency of the integration and example test packages. |
| `build-artifacts/` | Automatically created bindings, staged Python packages/native extensions, and temporary generated-code output. Ignored by Git; do not edit. |

Example tests live alongside the applications in [`example/tests/`](../example/tests/README.md):
`python/` contains the pytest cases, and `rust/` contains the Cargo package
`example_smoke_tests` and its executable probe.

The repository-root `conftest.py` uses `tests/preparation.py` to prepare pytest's
native dependencies once for both library and example tests, including combined
runs. `tests/integration_worker.py` runs the helper in `tests/python/integration_server.py`
in a disposable process; `tests/lifecycle_worker.py` checks worker disposal in its
own process. `tests/clean_artifacts.py` previews or removes old staged packages.
Library callback completion utilities live in `tests/python/callback_helpers.py`.
Conftest files contain hooks and fixtures; ordinary helpers are imported directly.
Rust unit tests still live beside their implementation in `crates/`, inside `#[cfg(test)]`
modules, and crate-level integration tests live in `crates/egui-states/tests/`.

## Prerequisites

Run commands from the repository root with Rust/Cargo and Python 3.12 or newer.
Use `uv run` for the commands below: it selects and prepares the project environment
on Windows, macOS, and Linux without requiring shell activation. You can also run
`uv sync` explicitly to prepare the development environment.
The selected interpreter must have pytest and NumPy. Native builds require the
platform C linker; Rust tests with the `python` feature also require the matching
Python shared library/development linker files. Tests use local TCP sockets.
Example builds additionally need the native GUI dependencies used by the examples.

## Rust: Cargo

Run the Rust library, macro, generated-consumer, and shared integration tests:

```sh
cargo test --locked --no-fail-fast -- --test-threads=1
```

These are normal Cargo tests. `--no-fail-fast` continues with other test binaries
when one fails. `--test-threads=1` keeps the large transfer tests sequential.
Cargo reports failures and returns its usual nonzero status. No Python test runner
or interpreter is required for this command.

The workspace's explicit default members are `egui_states`, `egui_states_macros`,
`egui_states_test_support`, and `egui_states_test_process`. The integration package
enables `server` and `build_scripts` on the library through its dependencies.
The same package defaults apply to bare `cargo build` and `cargo check`.

Additional feature configurations and the optional example suite run separately:

```sh
# Default client features
cargo test --locked -p egui_states
# Server without default client features
cargo test --locked -p egui_states --no-default-features --features server --lib
# Python-specific native signal tests, using uv's Python environment
uv run cargo test --locked -p egui_states --features "server build_scripts python" --lib -- --test-threads=1
# Rust examples
cargo test --locked -p example_smoke_tests
```

The Python-feature command requires the matching Python development/shared
library. PyO3 can discover the uv environment; set `PYO3_PYTHON` explicitly if you
need to choose a different interpreter. If the linker cannot find `libpython`,
install the matching development library or supply its directory in `LIBRARY_PATH`.

Use `cargo test --workspace` to select all workspace members, including the Python
extension and example crates, which have additional build prerequisites. Different
feature configurations still require separate Cargo invocations; one build cannot
exercise both enabled and disabled features.

## Python: pytest

```sh
uv run python -m pytest                      # Python library tests (default)
uv run python -m pytest example/tests/python -ra    # Python examples
uv run python -m pytest tests/python example/tests/python -ra # Both Python suites
```

With an activated, prepared virtual environment, use `python -m pytest` or `pytest`
directly. `uv run` normally prepares the environment first; `uv run --no-sync python
-m pytest` skips that environment synchronization once dependencies are installed.
Pytest still builds and stages a fresh extension.

Pytest owns Python behavior and Python binding import tests. It builds the native
extension, generated bindings, and Rust client probes needed to exercise Python,
but does not run Cargo's Rust unit tests or compiler-diagnostic test suite.

To focus on one area, use normal test selection:

```sh
cargo test --locked -p egui_states_test_support --test codegen
uv run python -m pytest tests/python/test_numpy.py
uv run python -m pytest -k callbacks
```

## Preparation and build settings

Both entry points respect Cargo configuration and target directories. Offline runs
require cached dependencies. For Bash/Zsh:

```sh
CARGO_NET_OFFLINE=true cargo test --locked -p egui_states_test_support --test codegen
CARGO_NET_OFFLINE=true uv run --offline --no-sync python -m pytest
CARGO_TARGET_DIR=/path/to/target cargo test --locked -p egui_states
```

For PowerShell, set the environment variable before the command:

```powershell
$env:CARGO_NET_OFFLINE = "true"
uv run --offline --no-sync python -m pytest
Remove-Item Env:CARGO_NET_OFFLINE
```

Set `CARGO_TARGET_DIR` the same way when choosing a custom build directory.

Pytest builds the native extension **before test collection**, using its current
interpreter. Cargo JSON artifacts locate the library and executables, including
custom target locations. A copy of the Python package and freshly built extension
is staged in ignored `tests/build-artifacts/runtime-*`. Preparation prints the loaded
extension path and build diagnostics. It does not overwrite package sources or
an installed extension. Subprocesses inherit that staged package via `PYTHONPATH`;
the integration worker also reports and checks its extension path. Do not import
`egui_states` before invoking pytest in the same Python process.

All 19 shared scenarios remain individually reported in Cargo and pytest. Their
single registration table is `tests/rust-integration/src/scenario_registry.rs`.
It generates Rust test wrappers and the list printed by the native probe:

```sh
cargo run --locked -p egui_states_test_support --bin client-probe -- --list-scenarios
```

Pytest reads that list during preparation and parametrizes the integration test
from it. Adding a scenario requires registering its Cargo name and probe ID in
that table and implementing its behavior in `src/scenarios.rs`; update the Rust
fixture and Python `integration_server.py` command handlers if needed. There is
no separate Python scenario list to maintain. A Cargo test also checks the probe's
listing against the registry. Test bindings require their generated layout version hash.
Python-independent signal semantics run with the normal server tests; the blocking
Python release regression runs explicitly in the Python-feature Rust layer.

The `*-wire` scenarios cross every value shape of the shared schema (including the
`wire_types` group) in both directions, so the Python server's runtime type
descriptions and encoder are checked against the Rust client, not only against
themselves. Their expected values are written three times: in `src/scenarios.rs`
(client), `src/fixture.rs`, and `integration_server.py` (servers); keep them equal.
`takeover` and `rejected-client` cover one-client-at-a-time behavior; for the
latter, both servers must also log a "wrong version" warning.

## Cleaning staged Python packages

Each pytest session keeps its staged package because loaded native libraries may
remain locked on Windows. Preview accumulated packages without building anything:

```sh
uv run --no-sync python -m tests.clean_artifacts
```

After all pytest sessions (including IDE runs) and their subprocesses have stopped:

```sh
uv run --no-sync python -m tests.clean_artifacts --delete
```

This removes only direct `runtime-*` directories under `tests/build-artifacts`.
Generated shared bindings and Cargo caches are retained. Symlinks are refused;
removal failures are reported with a nonzero exit status, and cleanup continues
with other packages. The command does not detect active sessions or prune
automatically. Cleanup behavior is checked against temporary directories by
`tests/python/test_artifact_cleanup.py`.

## Fixtures and failure containment

Local storage/conversion tests use an unstarted server. Callback ordering tests use
one worker and a queued completion marker. Concurrency tests use explicit gates and
release them in cleanup. Socket reservations use bind/close/rebind because the
production server has no bound-address accessor. A genuine intervening bind failure
is reported, never hidden by a retry.

Potentially blocking Rust tests execute in a child test process with a 25-second
deadline. Native probes have their own hard deadline and captured output. Python
shared integration servers and the worker-disposal regression run in disposable
processes so blocked or retained workers cannot affect the parent pytest process.
Subprocess errors and timeouts include their captured output. Cargo and pytest
report results directly; there is no combined runner or automatic aggregate log
directory.

Real packer tests use the production 10 MiB threshold and cover below/at/above it,
three chunks, partial tails, ACKs, publication boundaries, and subsequent transfers.
Run these sequentially (`--test-threads=1`); their peak memory is several hundred
MiB. Headless image tests inspect egui texture deltas and exact pixels without a GPU.

## Generated code and macro diagnostics

`tests/generated-code-consumer` is a test-only consumer crate shared as build input:

- Cargo discovers `tests/rust-integration/tests/codegen.rs`. It compiles and executes the
  generated Rust consumer and real custom derive, compiles the `awkward` schema,
  and checks public macro and conflicting-definition compiler diagnostics. These
  tests run without Python.
- Pytest discovers `tests/python/test_codegen.py`. It imports and instantiates
  generated Python, including one package per kind of awkward input.

`awkward.rs` holds valid Rust inputs that are hard for generators: Python keywords
as names, strings that need escaping, non-finite floats, and a unit struct. The
generated files themselves (determinism, no rewrite of unchanged files, ordering,
layout hashes, and schema validation panics) are checked once, in-process, by
`crates/egui-states/tests/build_scripts_codegen.rs`.

Nested Cargo builds from Rust tests use a dedicated `test-codegen` directory under
Cargo's active profile directory, so they respect custom target locations without
waiting on the parent build lock. Compiler cases serialize their child builds even
when Cargo's test harness runs tests concurrently. Their first run can take longer
while building this separate cache. The fixture's local lockfile is ignored; no
runtime dependencies are needed. The workspace lists the renamed test crate paths.

Exact text assertions are retained for stable ordering and layout/wire contracts;
ordinary semantic output checks use compiling and importing consumers instead.

## Known failures and scope

Production defects discovered by these tests remain **normal failures**. There are
no expected-failure markers hiding them; each test asserts the intended contract.
Every other failure is a regression. Current known failures:

| Test(s) | Defect |
| --- | --- |
| `server_core::core::tests::data_receiver_respects_message_count_limit` | Server batches hold eleven messages; the contract is at most ten. |
| `client::messages::serializer_tests::batches_hold_at_most_max_msg_count_messages_in_order` | Client batches hold up to twelve messages. |
| `client::data_take::lifecycle_tests::{removing,resetting}_pending_blocking_key_acknowledges_once`; scenarios `blocking-remove`, `blocking-reset` (Cargo and pytest) | Discarding a pending blocking `DataMultiTake` key does not acknowledge it, so the next key never progresses. |
| `server_core::signals::tests::reset_keeps_a_claimed_id_blocked`; `connection_tests::callbacks_for_one_state_do_not_overlap_across_a_reconnect` | A disconnect unblocks ids whose callbacks are still running, so one state's callbacks can run concurrently. |
| `server::data::tests::overflowing_data_ranges_are_rejected_without_changes`; `test_data.py::test_huge_data_indexes_raise_value_error` | `index + count` overflows in `Data`/`DataMulti` range checks and panics instead of returning an error. |
| `connection_tests::a_collection_larger_than_one_frame_reaches_the_client` | A whole-collection update is one message; above the 12 MiB frame limit the client drops the connection. |
| `connection_tests::client_reports_connected_only_after_the_server_accepts` | The client reports `Connected` as soon as it sends the handshake. |
| `connection_tests::connect_requested_right_after_build_is_not_lost` | `Client::connect` before the background task starts waiting is cleared and lost. |
| `client::value_vec::tests::malformed_replacement_keeps_the_previous_list`; `client::value_map::tests::malformed_replacement_keeps_the_previous_map` | A malformed full update leaves a partial collection. |
| codegen `state_empty` | `#[derive(State)]` accepts a struct without fields. |
| codegen `typed_serde_skip`, `typed_serde_flatten`, `typed_serde_untagged` | `#[typed]` accepts Serde attributes that change the encoding `Typed` describes. |
| `test_codegen.py::test_python_keywords_as_generated_names`, `test_special_literals_as_generated_initial_values`, `test_unit_struct_values` | The Python generator emits keywords as names, Rust `\u{..}` escapes, and `NaN`/`inf` literals; unit structs cannot be serialized (`__getstate__()` is `None`). |
| `test_values.py::test_invalid_value_inputs_preserve_previous_value[values.test_enum-2]` | A non-enum value for an enum state raises `AttributeError`. |
| `test_numpy.py::test_rejected_numeric_buffers_preserve_state[byteorder-*]` | Non-native byte-order buffers are accepted and stored byte-swapped. |
| `test_lifecycle.py::test_disposal_releases_workers` | Python signal workers have no exit path, so a released server keeps them. |

Some tests pin current behavior that has no written contract, and say so in their
comments: discarded pending client signals after a disconnect, client writes made
while disconnected, out-of-range client collection edits, and f32 rounding.

The test overhaul changes test infrastructure, schemas, test cases, and
`#[cfg(test)]` Rust code. A separately approved macro compatibility fix emits
negative enum discriminants as a minus operator followed by an integer literal,
avoiding a rust-analyzer parsing error. Its tests cover integer boundaries, wire
values, and both atomic derives. No `close()` tests, production API changes,
generator refactors, browser suite, packaging validation, or platform CI expansion
are included.
