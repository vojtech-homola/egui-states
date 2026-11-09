# ruff: noqa: PLC0415
"""Native worker regressions run outside pytest so leaked threads cannot accumulate."""

import sys
from tests.preparation import ROOT, run


def test_disposal_releases_workers(tmp_path):
    """Release callback workers when their owning servers are disposed."""
    # A different cwd ensures subprocess imports honor the staged PYTHONPATH.
    run([sys.executable, ROOT / "tests/lifecycle_worker.py"], cwd=tmp_path, timeout=15)


def test_heartbeat_settings_are_validated():
    """Accept valid or disabled heartbeat settings and reject invalid ones."""
    import pytest
    from egui_states_test_bindings import StatesServer  # ty: ignore[import-not-found]

    StatesServer(signals_workers=1, heartbeat_interval=0.5, heartbeat_timeout=2.0)
    # A disabled heartbeat ignores its interval.
    StatesServer(signals_workers=1, heartbeat_interval=0.0, heartbeat_timeout=None)
    for interval, timeout in [
        (0.0, 1.0),
        (-1.0, 1.0),
        (2.0, 2.0),
        (2.0, 0.5),
        (float("nan"), 1.0),
        (1.0, float("inf")),
    ]:
        with pytest.raises(ValueError, match="heartbeat"):
            StatesServer(signals_workers=1, heartbeat_interval=interval, heartbeat_timeout=timeout)


def test_server_restarts_on_a_different_port():
    """Restart a server with a new port and release the previous listener."""
    import socket
    import pytest
    from egui_states_test_bindings import StatesServer  # ty: ignore[import-not-found]
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


def test_server_start_while_running_requires_the_active_settings():
    """Accept a repeated start only with the address and token in use."""
    import pytest
    from egui_states_test_bindings import StatesServer  # ty: ignore[import-not-found]

    server = StatesServer(signals_workers=1)
    try:
        assert server.address() is None
        server.start(0, (127, 0, 0, 1), "token")
        address = server.address()
        assert address is not None
        ip_addr, port = address
        assert ip_addr == (127, 0, 0, 1)
        assert port != 0

        server.start(0, (127, 0, 0, 1), "token")
        server.start(port, (127, 0, 0, 1), "token")
        # No address means loopback, which is the address in use.
        server.start(port, None, "token")
        with pytest.raises(RuntimeError, match="already running on 127.0.0.1"):
            server.start(port, (0, 0, 0, 0), "token")
        with pytest.raises(RuntimeError, match="different token"):
            server.start(port, (127, 0, 0, 1), "other")
        assert server.address() == address
    finally:
        server.stop()
    assert server.address() is None


def test_server_serves_web_page_on_its_port(tmp_path):
    """Serve the configured web page on the server port and reload or stop it."""
    import urllib.request
    import pytest
    from egui_states_test_bindings import StatesServer  # ty: ignore[import-not-found]
    from tests.python_test_helpers.network import free_port

    without_page = StatesServer(signals_workers=1)
    assert without_page.web_path() is None
    with pytest.raises(RuntimeError, match="web_path"):
        without_page.start_web()
    assert not without_page.is_web_running()

    page = tmp_path / "page"
    page.mkdir()
    index = page / "index.html"
    index.write_text("first")
    port = free_port()

    def read(path: str) -> bytes:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}{path}", timeout=5) as response:
            assert response.status == 200
            return response.read()

    server = StatesServer(signals_workers=1, web_path=page)
    try:
        assert server.web_path() == page.absolute()
        assert not server.is_web_running()
        server.start_web()
        assert server.is_web_running()
        server.start(port, (127, 0, 0, 1))
        assert read("/") == b"first"

        index.write_text("second")
        server.start_web()
        assert read("/index.html") == b"second"

        index.unlink()
        with pytest.raises(ValueError, match="index.html"):
            server.start_web()
        assert read("/") == b"second"

        index.write_text("third")
        server.stop_web()
        assert not server.is_web_running()
        assert server.web_path() == page.absolute()
        server.start_web()
        assert read("/") == b"third"
    finally:
        server.stop()
