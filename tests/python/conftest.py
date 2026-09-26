"""Generate independent bindings before test-module collection."""

import os
from pathlib import Path
import sys

import pytest
from tests.python_test_helpers.network import free_port

ROOT = Path(__file__).resolve().parents[2]
GENERATED = ROOT / "tests" / "build-artifacts"


def pytest_configure(config):
    """Generate bindings and locate the native client from Cargo artifacts."""
    from tests.preparation import build, executable, run

    try:
        artifacts = build("egui_states_test_support", bins=True)
        run([executable(artifacts, "prepare-test-bindings")], timeout=60)
        config._client_probe = executable(artifacts, "client-probe")
        config._client_scenarios = run([config._client_probe, "--list-scenarios"], timeout=5).splitlines()
        if not config._client_scenarios or len(set(config._client_scenarios)) != len(config._client_scenarios):
            raise RuntimeError(f"Probe reported an empty or duplicate scenario list: {config._client_scenarios!r}")
    except (RuntimeError, OSError, KeyError) as error:
        raise pytest.UsageError(f"Fixture preparation failed: {error}") from error
    sys.path.insert(0, str(GENERATED))
    os.environ["PYTHONPATH"] = os.pathsep.join([str(GENERATED), os.environ.get("PYTHONPATH", "")])


def pytest_generate_tests(metafunc):
    """Use the same scenario registration that generates Cargo's test functions."""
    if "native_scenario" in metafunc.fixturenames:
        metafunc.parametrize("native_scenario", metafunc.config._client_scenarios)


def _bundle(running):
    from egui_states_test_bindings import StatesServer
    from egui_states.logging import LogLevel

    errors = []
    server = StatesServer(signals_workers=1, error_handler=errors.append)
    server.logging.add_logger(LogLevel.Error, lambda message: errors.append(RuntimeError(message)))
    try:
        if running:
            port = free_port()
            try:
                server.start(port, (127, 0, 0, 1))
            except OSError as error:
                raise RuntimeError(
                    f"Failed to bind test server on 127.0.0.1:{port}; reservation was released before start"
                ) from error
        yield server, server.states, errors
    finally:
        server.stop()
        assert not errors, f"Unexpected server/callback errors: {errors!r}"


@pytest.fixture
def local_bundle():
    """Local conversions and storage need neither a listener nor worker threads."""
    yield from _bundle(False)


@pytest.fixture
def server_bundle():
    """One worker gives callback tests a deterministic queued completion marker."""
    yield from _bundle(True)
