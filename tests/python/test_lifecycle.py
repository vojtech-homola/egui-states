# ruff: noqa: PLC0415
"""Native worker regressions run outside pytest so leaked threads cannot accumulate."""

import sys
from typing import Any, cast

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


def test_transfer_profiles_are_validated():
    """Accept every transfer profile and reject anything else."""
    import pytest
    from egui_states import TransferProfile
    from egui_states_test_bindings import StatesServer  # ty: ignore[import-not-found]

    for profile in TransferProfile:
        StatesServer(signals_workers=1, transfer_profile=profile)
    # Values an untyped caller might pass.
    for profile in ["balanced", "Balanced", 3, -1]:
        with pytest.raises(ValueError, match="TransferProfile"):
            StatesServer(signals_workers=1, transfer_profile=cast(Any, profile))


def test_client_policies_are_validated():
    """Accept every client policy and reject anything else."""
    import pytest
    from egui_states import ClientPolicy
    from egui_states_test_bindings import StatesServer  # ty: ignore[import-not-found]

    for policy in ClientPolicy:
        StatesServer(signals_workers=1, client_policy=policy)
    # Values an untyped caller might pass.
    for policy in ["reject", "Reject", 2, -1]:
        with pytest.raises(ValueError, match="ClientPolicy"):
            StatesServer(signals_workers=1, client_policy=cast(Any, policy))


def test_connect_request_timeouts_are_validated():
    """Accept a positive connect request timeout and reject anything else."""
    import pytest
    from egui_states_test_bindings import StatesServer  # ty: ignore[import-not-found]

    StatesServer(signals_workers=1, connect_request_timeout=0.5)
    for timeout in [0.0, -1.0, float("nan"), float("inf")]:
        with pytest.raises(ValueError, match="connect request timeout"):
            StatesServer(signals_workers=1, connect_request_timeout=timeout)


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


def test_tls_servers_report_their_fingerprint_and_speak_tls13(tmp_path):
    """Present a generated certificate whose fingerprint the server reports."""
    import hashlib
    import socket
    import ssl
    import pytest
    from egui_states_test_bindings import StatesServer  # ty: ignore[import-not-found]
    from tests.python_test_helpers.network import free_port

    assert StatesServer(signals_workers=1).tls_fingerprint() is None

    def read_all(connection: socket.socket) -> bytes:
        response = b""
        while chunk := connection.recv(4096):
            response += chunk
        return response

    port = free_port()
    server = StatesServer(signals_workers=1, tls=True)
    fingerprint = server.tls_fingerprint()
    assert fingerprint is not None
    assert len(fingerprint) == 64
    assert fingerprint == fingerprint.lower()
    int(fingerprint, 16)
    try:
        server.start(port, (127, 0, 0, 1))
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        context.check_hostname = False
        context.verify_mode = ssl.CERT_NONE
        with (
            socket.create_connection(("127.0.0.1", port), timeout=5) as raw,
            context.wrap_socket(raw, server_hostname="localhost") as connection,
        ):
            assert connection.version() == "TLSv1.3"
            certificate = connection.getpeercert(binary_form=True)
            assert certificate is not None
            assert hashlib.sha256(certificate).hexdigest() == fingerprint

        # A browser that opens the http:// address is sent to https://.
        with socket.create_connection(("127.0.0.1", port), timeout=5) as plain:
            plain.sendall(f"GET /page?x=1 HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n".encode())
            response = read_all(plain)
        assert response.startswith(b"HTTP/1.1 307"), response
        assert f"\r\nLocation: https://localhost:{port}/page?x=1\r\n".encode() in response

        # The certificate stays the same across a restart.
        server.stop()
        server.start(port, (127, 0, 0, 1))
        assert server.tls_fingerprint() == fingerprint
    finally:
        server.stop()

    garbage = tmp_path / "garbage.pem"
    garbage.write_text("not a certificate")
    with pytest.raises(ValueError, match="together"):
        StatesServer(signals_workers=1, tls=True, tls_cert=garbage)
    with pytest.raises(ValueError, match="require tls"):
        StatesServer(signals_workers=1, tls_cert=garbage, tls_key=garbage)
    with pytest.raises(ValueError, match="no PEM certificate"):
        StatesServer(signals_workers=1, tls=True, tls_cert=garbage, tls_key=garbage)
    with pytest.raises(FileNotFoundError, match="missing.pem"):
        StatesServer(signals_workers=1, tls=True, tls_cert=tmp_path / "missing.pem", tls_key=garbage)


def test_tls_servers_present_a_supplied_certificate(tmp_path):
    """Report the fingerprint of a certificate created with OpenSSL."""
    import hashlib
    import shutil
    import ssl
    import subprocess
    import pytest
    from egui_states_test_bindings import StatesServer  # ty: ignore[import-not-found]

    openssl = shutil.which("openssl")
    if openssl is None:
        pytest.skip("openssl is not installed")
    cert, key = tmp_path / "cert.pem", tmp_path / "key.pem"
    subprocess.run(
        [openssl, "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1"]
        + ["-nodes", "-keyout", key, "-out", cert, "-days", "1", "-subj", "/CN=localhost"],
        check=True,
        capture_output=True,
    )
    server = StatesServer(signals_workers=1, tls=True, tls_cert=cert, tls_key=str(key))
    expected = hashlib.sha256(ssl.PEM_cert_to_DER_cert(cert.read_text())).hexdigest()
    assert server.tls_fingerprint() == expected


def test_browser_pages_from_other_origins_cannot_connect():
    """Refuse WebSocket upgrades from foreign pages and validate the allow lists."""
    import socket
    import pytest
    from egui_states_test_bindings import StatesServer  # ty: ignore[import-not-found]
    from tests.python_test_helpers.network import free_port

    with pytest.raises(ValueError, match="allowed origin"):
        StatesServer(signals_workers=1, allowed_origins=["localhost:8090"])

    def upgrade_status(origin: str) -> str:
        request = (
            f"GET /ws HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\n"
            "Connection: Upgrade\r\nSec-WebSocket-Version: 13\r\n"
            f"Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nOrigin: {origin}\r\n\r\n"
        )
        with socket.create_connection(("127.0.0.1", port), timeout=5) as connection:
            connection.sendall(request.encode())
            response = b""
            while b"\r\n" not in response:
                chunk = connection.recv(1024)
                if not chunk:
                    break
                response += chunk
        return response.split(b"\r\n", 1)[0].decode()

    port = free_port()
    server = StatesServer(signals_workers=1, allowed_origins=("http://localhost:*",))
    try:
        server.start(port, (127, 0, 0, 1))
        assert upgrade_status("https://news.example").startswith("HTTP/1.1 403")
        assert upgrade_status(f"http://127.0.0.1:{port}").startswith("HTTP/1.1 101")
        assert upgrade_status("http://localhost:8090").startswith("HTTP/1.1 101")
    finally:
        server.stop()


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

    # File system failures raise OSError subclasses; unservable pages ValueError.
    missing = StatesServer(signals_workers=1, web_path=tmp_path / "missing")
    with pytest.raises(FileNotFoundError, match="missing"):
        missing.start_web()
    not_directory = tmp_path / "file.txt"
    not_directory.write_text("x")
    with pytest.raises(ValueError, match="not a directory"):
        StatesServer(signals_workers=1, web_path=not_directory).start_web()

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
