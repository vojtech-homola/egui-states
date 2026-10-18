# Tests

Run everything from the repository root. You need Rust and Python 3.12+ with
pytest and NumPy; `uv run` prepares the environment. Tests use local TCP sockets.

## Running

```sh
# Rust: library, macros, generated-code consumer, shared integration scenarios
cargo test --locked --no-fail-fast -- --test-threads=1

# Python: library tests (default), examples, or both
uv run python -m pytest
uv run python -m pytest example/tests/python -ra
uv run python -m pytest tests/python example/tests/python -ra
```

`--test-threads=1` keeps the large transfer tests sequential (they peak at a few
hundred MiB). Feature configurations need separate Cargo runs:

```sh
cargo test --locked -p egui_states                                              # client defaults
cargo test --locked -p egui_states --no-default-features --features server --lib
uv run cargo test --locked -p egui_states --features "server build_scripts python" --lib -- --test-threads=1
cargo test --locked -p example_smoke_tests                                      # Rust examples
```

The experimental C ABI is not part of the default packages and has its own runs:

```sh
cargo test --locked -p egui_states --no-default-features --features c_abi --lib -- --test-threads=1
cargo test --locked -p egui_states --features "build_scripts c_abi" --test build_scripts_c
cargo test --locked -p egui_states_c -- --test-threads=1   # also checks include/egui_states.h
cargo test --locked -p counter_c                           # C counter example; needs a C and C++ compiler
```

Regenerate the header with `EGUI_STATES_BLESS=1 cargo test -p egui_states_c --test header`.
Set `EGUI_STATES_SKIP_C_TESTS=1` to skip the C compiler test.

The `python` feature links against `libpython`. If the linker cannot find it,
install the Python development package or put its directory in `LIBRARY_PATH`;
set `PYO3_PYTHON` to choose another interpreter.

Select tests as usual, e.g. `cargo test -p egui_states_test_support --test codegen`
or `uv run python -m pytest -k callbacks`. Cargo settings such as
`CARGO_TARGET_DIR` and `CARGO_NET_OFFLINE=true` apply to both runners; for offline
pytest add `uv run --offline --no-sync`.

## Layout

| Folder | Contents |
| --- | --- |
| `python/` | Python library tests. |
| `python_test_helpers/` | Port reservations and bounded waits, shared with example tests. |
| `rust-integration/` | Integration and compiler tests, shared scenarios, and the `client-probe` used by pytest (`egui_states_test_support`). |
| `shared-test-states/` | State definitions that generate the Rust and Python test bindings (`egui_states_test_schema`). |
| `generated-code-consumer/` | Crate that compiles generated bindings, awkward inputs (`awkward.rs`), and invalid macro inputs. |
| `rust-process-helpers/` | Subprocess deadlines and captured output (`egui_states_test_process`). |
| `build-artifacts/` | Generated bindings and staged packages. Ignored by Git; do not edit. |

Also: Rust unit tests sit in `#[cfg(test)]` modules in `crates/`, crate integration
tests in `crates/egui-states/tests/`, and example tests in
[`example/tests/`](../example/tests/README.md).

## How pytest prepares

Before collection, the root `conftest.py` (via `tests/preparation.py`) builds the
native extension with the current interpreter and stages it with the Python
package in `tests/build-artifacts/runtime-*`. Subprocesses get it through
`PYTHONPATH`. Package sources and installed extensions are never overwritten.
Do not import `egui_states` before starting pytest in the same process.

Staged packages are kept, because Windows may lock loaded libraries. Once no
pytest session is running:

```sh
uv run --no-sync python -m tests.clean_artifacts           # preview
uv run --no-sync python -m tests.clean_artifacts --delete  # remove runtime-* only
```

## Shared scenarios

The 19 client/server scenarios run against both the Rust and the Python server and
are reported individually by Cargo and pytest. They are registered once in
`rust-integration/src/scenario_registry.rs`; pytest reads the list from
`client-probe --list-scenarios`. To add one, register it there, implement it in
`src/scenarios.rs`, and extend `src/fixture.rs` and `python/integration_server.py`
if needed.

The `*-wire` scenarios send every value shape of the shared schema both ways. Their
expected values live in `src/scenarios.rs`, `src/fixture.rs`, and
`integration_server.py`; keep all three equal.

## Generated code

- `rust-integration/tests/codegen.rs` compiles and runs the generated Rust consumer
  and checks macro compiler diagnostics (no Python needed). Its nested builds use a
  separate `test-codegen` target directory, so the first run is slower.
- `python/test_codegen.py` imports the generated Python, one package per awkward
  input: Python keywords as names, strings that need escaping, non-finite floats,
  and a unit struct.
- `crates/egui-states/tests/build_scripts_codegen.rs` checks the generated files
  themselves: determinism, ordering, layout hashes, and validation panics.
  `build_scripts_c.rs` does the same for the experimental C header (with
  `c_abi`) and parses its type strings with the C ABI.

## Conventions

- Tests that could block run in a child process with a deadline (25 s for Rust
  tests); Python integration servers and the worker-disposal check run in
  disposable processes. Timeouts report captured output.
- There are no expected-failure markers: a test asserts the intended contract, and
  any failure is a real defect. There are currently no known failures.
- Some tests pin behavior without a written contract and say so in a comment:
  pending client signals dropped on disconnect, client writes while disconnected,
  out-of-range client collection edits, and f32 rounding.
