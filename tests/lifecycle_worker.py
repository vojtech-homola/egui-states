"""Check native worker disposal in a disposable process."""

import gc
import threading
import time
import logging
from egui_states_test_bindings import StatesServer  # ty: ignore[import-not-found]


class _Errors(logging.Handler):
    def __init__(self, errors: list) -> None:
        super().__init__(logging.ERROR)
        self.errors = errors

    def emit(self, record: logging.LogRecord) -> None:
        self.errors.append(RuntimeError(record.getMessage()))


errors = []
logging.getLogger("egui_states").addHandler(_Errors(errors))
before = set(threading.enumerate())
for _ in range(3):
    server = StatesServer(signals_workers=2, error_handler=errors.append)
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
