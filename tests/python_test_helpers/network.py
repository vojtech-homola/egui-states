"""Loopback port reservations for library and example tests."""

import socket


def free_port() -> int:
    """Reserve and release a loopback port before the server binds it."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        sock.listen(1)
        return int(sock.getsockname()[1])
