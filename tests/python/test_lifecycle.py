# ruff: noqa: PLC0415
"""Native worker regressions run outside pytest so leaked threads cannot accumulate."""

import sys
from tests.preparation import ROOT, run


def test_disposal_releases_workers(tmp_path):
    """Release callback workers when their owning servers are disposed."""
    # A different cwd ensures subprocess imports honor the staged PYTHONPATH.
    run([sys.executable, ROOT / "tests/lifecycle_worker.py"], cwd=tmp_path, timeout=15)


def test_server_restarts_on_a_different_port():
    """Restart a server with a new port and release the previous listener."""
    import socket
    import pytest
    from egui_states_test_bindings import StatesServer  # ty: ignore[unresolved-import]
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


def test_server_serves_web_page_on_its_port(tmp_path):
    import urllib.request
    import pytest
    from egui_states_test_bindings import StatesServer
    from tests.python_test_helpers.network import free_port

    first, second = tmp_path / "first", tmp_path / "second"
    for directory in (first, second):
        directory.mkdir()
        (directory / "index.html").write_text(directory.name)

    port = free_port()
    server = StatesServer(signals_workers=1)
    try:
        assert server.web_path() is None
        server.serve_web(first)
        server.start(port, (127, 0, 0, 1))
        assert server.web_path() == first.absolute()
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=5) as response:
            assert response.status == 200
            assert response.read() == b"first"

        with pytest.raises(ValueError, match="index.html"):
            server.serve_web(tmp_path)
        server.serve_web(str(second))
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/index.html", timeout=5) as response:
            assert response.read() == b"second"

        server.stop_web()
        assert server.web_path() is None
    finally:
        server.stop()
