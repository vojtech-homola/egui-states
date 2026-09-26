"""Ensure each documented server command stays alive and cleans up on Ctrl+C."""

import os
import queue
import signal
import socket
import subprocess
import sys
import threading
import time

import pytest

from tests.preparation import ROOT
from tests.python_test_helpers.network import free_port


@pytest.mark.skipif(os.name == "nt", reason="This subprocess SIGINT check uses POSIX signal delivery")
@pytest.mark.parametrize("example", ["counter", "showcase"])
@pytest.mark.parametrize("language", ["python", "rust"])
def test_entrypoint_shutdown(request, example, language):
    """Verify each server command exits cleanly and releases its port on SIGINT."""
    port = free_port()
    if language == "python":
        command = [sys.executable, "-u", str(ROOT / "example" / example / "python" / "run.py")]
    else:
        command = [str(request.config._example_servers[example])]
    command.extend(["--port", str(port)])
    process = subprocess.Popen(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    if process.stdout is None:
        pytest.skip("Failed to capture process output")
    output = []
    lines = queue.Queue()

    def read_output():
        if process.stdout is None:
            pytest.skip("Failed to capture process output")

        for line in process.stdout:
            output.append(line)
            lines.put(line)

    reader = threading.Thread(target=read_output, daemon=True)
    reader.start()
    try:
        deadline = time.monotonic() + 10
        while True:
            remaining = deadline - time.monotonic()
            assert remaining > 0, f"No readiness announcement: {''.join(output)}"
            try:
                line = lines.get(timeout=min(remaining, 0.1))
            except queue.Empty:
                assert process.poll() is None, f"Exited before readiness: {''.join(output)}"
                continue
            if f"server listening on {port}" in line:
                break
        assert process.poll() is None, "Server entry point exited instead of waiting"
        process.send_signal(signal.SIGINT)
        assert process.wait(timeout=10) == 0, "".join(output)
        reader.join(timeout=1)
        # A normal stop releases the listener immediately.
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", port))
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)
        reader.join(timeout=1)
        process.stdout.close()
