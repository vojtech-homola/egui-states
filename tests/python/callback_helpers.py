"""Completion markers for the library fixture's single callback worker."""

import threading
from tests.python_test_helpers.waiting import wait_event


def drain_callbacks(states):
    """Wait behind already queued changes on the fixture's sole worker."""
    event = threading.Event()
    sentinel = states.integration.command
    sentinel.connect(lambda value: event.set() if value == 987654 else None)
    try:
        sentinel.set(987654)
        wait_event(event, 5)
    finally:
        sentinel.disconnect_all()
