"""Opt-in example bindings and native smoke probe preparation."""

import importlib.util
from pathlib import Path
import sys

import pytest

ROOT = Path(__file__).resolve().parents[3]


def pytest_configure(config):
    """Resolve all example binaries from Cargo's compiler-artifact messages."""
    from tests.preparation import build, executable  # ruff: ignore[import-outside-top-level]

    try:
        artifacts = build(
            "counter_gui", "showcase_gui", "example_smoke_tests", "showcase_server", "counter_server", bins=True
        )
        config._example_probe = executable(artifacts, "example-probe")
        config._example_servers = {name: executable(artifacts, f"{name}-server") for name in ("counter", "showcase")}
    except (RuntimeError, OSError, KeyError) as error:
        raise pytest.UsageError(f"Example preparation failed: {error}") from error


@pytest.fixture(params=["showcase", "counter"])
def example_server(request):
    """Yield a configured example server and check callback errors on teardown."""
    name = request.param
    directory = ROOT / "example" / name / "python"
    sys.path.insert(0, str(directory))
    try:
        spec = importlib.util.spec_from_file_location(f"{name}_example", directory / "run.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        errors = []
        server = module.setup_server(error_handler=errors.append)
        from egui_states.logging import LogLevel

        server.logging.add_logger(LogLevel.Error, lambda message: errors.append(RuntimeError(message)))
        assert not server.is_running(), "Import/setup must not start a server"
        try:
            yield name, module, server
        finally:
            server.stop()
            assert not errors, f"Unexpected example callback errors: {errors!r}"
    finally:
        sys.path.remove(str(directory))
