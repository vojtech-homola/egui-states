import threading
import traceback
from collections.abc import Callable

from egui_states.typing import SteteServerCoreBase


class SignalsManager:
    """The class for managing signals."""

    def __init__(
        self,
        server: SteteServerCoreBase,
        workers: int,
        error_handler: Callable[[Exception], None] | None,
    ):
        """Initialize the SignalsManager."""
        self._callbacks: dict[int, list[Callable]] = {}
        self._server = server

        self._workers_count = workers
        self._workers: list[threading.Thread] = []
        self._error_handler = error_handler or self._default_error_handler

    def start_manager(self) -> None:
        """Start the signals manager."""
        if self._workers:
            return

        for i in range(self._workers_count):
            worker = threading.Thread(target=self._run, daemon=True, name=f"signals_worker_{i}")
            self._workers.append(worker)
            worker.start()

    def _run(self) -> None:
        last_id: int | None = None
        while True:
            last_id, arg = self._server.value_get_signal(last_id)
            callbacks = self._callbacks.get(last_id, None)
            if callbacks:
                for callback in callbacks:
                    try:
                        if arg == ():
                            callback()
                        else:
                            callback(arg)
                    except Exception as e:
                        try:
                            self._error_handler(e)
                        except Exception:  # safety
                            pass
            else:
                error = IndexError(f"Signal with index {last_id} not found.")
                self._error_handler(error)

    @staticmethod
    def _default_error_handler(_e: Exception) -> None:
        traceback.print_exc()

    def set_error_handler(self, error_handler: Callable[[Exception], None] | None) -> None:
        """Set custom error handler."""
        self._error_handler = error_handler or self._default_error_handler

    def add_callback(self, value_id: int, callback: Callable) -> None:
        """Add a callback to a signal."""
        if value_id in self._callbacks:
            self._callbacks[value_id].append(callback)
        else:
            self._callbacks[value_id] = [callback]
        self._server.value_set_register(value_id, True)

    def remove_callback(self, value_id: int, callback: Callable) -> None:
        """Remove a callback from a signal."""
        if value_id in self._callbacks:
            if callback in self._callbacks[value_id]:
                self._callbacks[value_id].remove(callback)
                if not self._callbacks[value_id]:
                    self._server.value_set_register(value_id, False)

    def clear_callbacks(self, value_id: int) -> None:
        """Clear all callbacks from a signal."""
        if value_id in self._callbacks:
            self._callbacks[value_id].clear()
            self._server.value_set_register(value_id, False)
