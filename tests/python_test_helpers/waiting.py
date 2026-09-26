"""Bounded waits for observable test completion."""

import threading
import time


def wait_until(predicate, timeout: float = 5.0) -> None:
    """Wait for an observable condition, failing at the completion deadline."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.01)
    assert predicate(), f"condition did not complete within {timeout}s"


def wait_event(event: threading.Event, timeout: float = 5.0) -> None:
    """Wait for a callback event within its completion deadline."""
    assert event.wait(timeout), "timed out waiting for callback"
