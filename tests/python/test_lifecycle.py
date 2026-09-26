"""Native worker regressions run outside pytest so leaked threads cannot accumulate."""

import sys
from tests.preparation import ROOT, run


def test_disposal_releases_workers(tmp_path):
    # A different cwd ensures subprocess imports honor the staged PYTHONPATH.
    run([sys.executable, ROOT / "tests/lifecycle_worker.py"], cwd=tmp_path, timeout=15)


def test_server_restarts_on_a_different_port():
    import socket
    import pytest
    from egui_states_test_bindings import StatesServer
    from egui_states.logging import LogLevel

    # Hold both reservations together so the ports are guaranteed distinct.
    with socket.socket() as first, socket.socket() as second:
        first.bind(("127.0.0.1", 0))
        second.bind(("127.0.0.1", 0))
        first_port, second_port = first.getsockname()[1], second.getsockname()[1]
    errors = []
    server = StatesServer(signals_workers=1, error_handler=errors.append)
    server.logging.add_logger(LogLevel.Error, lambda message: errors.append(RuntimeError(message)))
    try:
        server.start(first_port, (127, 0, 0, 1), "first-token")
        assert server.is_running()
        server.stop()
        server.start(second_port, (127, 0, 0, 1), "second-token")
        assert server.is_running()
        with socket.socket() as probe:
            with pytest.raises(OSError):
                probe.bind(("127.0.0.1", second_port))
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", first_port))
    finally:
        server.stop()
    assert not errors, errors
