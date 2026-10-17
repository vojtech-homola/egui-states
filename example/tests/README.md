# Example tests

These suites exercise the actual counter and showcase application setup,
client/server communication, and server entry points.

| Folder | Purpose |
| --- | --- |
| `python/` | Pytest cases for Python example setup and communication, plus Python and Rust server command shutdown. |
| `rust/` | Cargo tests for Rust example setup and communication, plus the native client probe used by both suites. Package: `example_smoke_tests`. |

Run from the repository root:

```sh
uv run python -m pytest example/tests/python -ra
cargo test --locked -p example_smoke_tests
```

With a prepared, activated Python environment, `python -m pytest` or `pytest` can
replace `uv run python -m pytest`. Both suites require Cargo, native example build
dependencies, and local TCP sockets. Python tests also require pytest and NumPy.
See the [test guide](../../tests/README.md) for complete prerequisites, offline
settings, and custom Cargo target directories.

The repository-root `conftest.py` shares fresh-extension preparation with the
library tests through `tests/preparation.py`. The local Python `conftest.py` builds
example bindings and locates the native probes through Cargo's artifact output.
Rust tests reuse subprocess deadlines and output capture from
the `egui_states_test_process` dev-dependency in `tests/rust-process-helpers/`.
Python tests import shared port/wait utilities from `tests/python_test_helpers/`.
Example tests are excluded from the workspace's default Cargo package selection;
the explicit command above continues to select them.

Default pytest discovery still selects the library tests only. To run both Python
suites together:

```sh
uv run python -m pytest tests/python example/tests/python -ra
```
