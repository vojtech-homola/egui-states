"""A minimal counter server for the shared native GUI."""

import argparse
from collections.abc import Callable
import signal
import threading

from counter_bindings import StatesServer


def setup_server(*, error_handler: Callable[[Exception], None] | None = None) -> StatesServer:
    """Create the server, log edits, and reset the count on request.

    Args:
        error_handler (Callable[[Exception], None] | None, optional): Handler
            for asynchronous callback errors; None uses the default handler.

    Returns:
        StatesServer: Configured server; call start to listen for a client.
    """
    server = StatesServer(error_handler=error_handler)
    states = server.states
    states.count.set(0)
    states.count.connect(lambda value: print(f"count: {value}"))
    states.reset.connect(lambda: states.count.set(0, update=True))
    return server


def main() -> None:
    """Listen until interrupted, then stop the server."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=8092)
    args = parser.parse_args()
    stopped = threading.Event()
    signal.signal(signal.SIGINT, lambda *_: stopped.set())
    signal.signal(signal.SIGTERM, lambda *_: stopped.set())
    server = setup_server()
    try:
        server.start(args.port, (127, 0, 0, 1))
        print(f"Counter server listening on {args.port}", flush=True)
        # Another thread may take the signal, which only flags it for the
        # main thread; a timed wait lets the main thread run the handler.
        while not stopped.wait(0.5):
            pass
    finally:
        server.stop()


if __name__ == "__main__":
    main()
