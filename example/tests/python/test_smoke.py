"""Exercise actual example setup and actions, independently of library tests."""

import subprocess

import numpy as np

from tests.python_test_helpers.network import free_port
from tests.python_test_helpers.waiting import wait_until


def test_example_with_native_client(request, example_server):
    """Verify example defaults and actions through a native client connection."""
    name, module, server = example_server
    states = server.states
    observed = []
    count = states.count if name == "counter" else states.values.count
    count.connect(observed.append)
    if name == "counter":
        assert count.get() == 0
        count.set(-1)  # A non-default readiness marker for the headless probe.
    if name == "showcase":
        assert states.value_vec.items.get() == module.DEFAULT_VEC
        assert states.value_map.items.get() == module.DEFAULT_MAP
        image = states.image.image.get()
        np.testing.assert_array_equal(image[10, 20], [20, 10, 15, 255])
        np.testing.assert_array_equal(image[100, 100], [0, 220, 0, 255])
        assert states.image.images.indices() == [2, 7]
    port = free_port()
    server.start(port, (127, 0, 0, 1))
    result = subprocess.run(
        [str(request.config._example_probe), str(port), name],
        check=False,
        capture_output=True,
        text=True,
        timeout=30,
    )
    assert result.returncode == 0, f"{name}:\n{result.stdout}\n{result.stderr}"
    wait_until(lambda: observed == ([17, 42] if name == "counter" else [42]))
    assert count.get() == (0 if name == "counter" else 42)
    server.stop()
    assert not server.is_running()
