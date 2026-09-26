"""Run the same native client scenarios as the generated Rust server tests."""

import sys
from tests.preparation import ROOT, run


def test_python_server_with_native_client(request, native_scenario):
    """Verify the Python server against a shared native client scenario."""
    run(
        [sys.executable, ROOT / "tests/integration_worker.py", request.config._client_probe, native_scenario],
        timeout=35,
    )
