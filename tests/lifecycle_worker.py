"""Check native worker disposal in a disposable process."""

import gc
import threading
import time
from egui_states_test_bindings import StatesServer
from egui_states.logging import LogLevel

errors = []
before = set(threading.enumerate())
for _ in range(3):
    server = StatesServer(signals_workers=2, error_handler=errors.append)
    server.logging.add_logger(LogLevel.Error, lambda message: errors.append(RuntimeError(message)))
    server.start(0, (127, 0, 0, 1))
    server.stop()
    # stop is restartable; disposal, not stop alone, is the contract under test.
    del server
    gc.collect()
deadline = time.monotonic() + 3
while time.monotonic() < deadline:
    remaining = [t for t in threading.enumerate() if t not in before and t.name.startswith("signals_worker_")]
    if not remaining:
        break
    gc.collect()
    time.sleep(0.02)
assert not errors, errors
assert not remaining, (
    f"Workers retained after stopping and releasing 3 servers: {[(t.name, t.ident) for t in remaining]}"
)
