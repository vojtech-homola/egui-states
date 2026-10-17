"""Internal callback dispatch used by generated Python state servers."""

import threading
import traceback
import weakref
from collections.abc import Callable
from typing import Any

from egui_states._core import StateServerCore


def _run(manager_ref: "weakref.ref[SignalsManager]", server: StateServerCore) -> None:
    """Claim native signal events and dispatch their callbacks.

    A worker holds its manager only weakly while it waits, so releasing the
    server releases the manager; its finalizer then closes the native signals
    and the worker exits.

    Args:
        manager_ref (weakref.ref[SignalsManager]): The manager that owns the
            callbacks.
        server (StateServerCore): Native server from which events are claimed.
    """
    last_id: int | None = None
    while True:
        try:
            event = server.signal_get(last_id)
        except Exception as e:
            manager = manager_ref()
            if manager is None:
                return
            manager._error_handler(RuntimeError(f"Error while getting signal from server: {e}"))
            del manager
            continue
        if event is None:
            return
        manager = manager_ref()
        if manager is None:
            return
        last_id, arg, has_previous, previous = event
        manager._dispatch(last_id, arg, has_previous, previous)
        del manager, event, arg, previous


class SignalsManager:
    """Dispatch native signal events to callbacks on background workers."""

    def __init__(
        self,
        server: StateServerCore,
        workers: int,
        error_handler: Callable[[Exception], None] | None,
    ) -> None:
        """Initialize the signal manager.

        Args:
            server (StateServerCore): Native server from which events are
                claimed.
            workers (int): Number of callback worker threads.
            error_handler (Callable[[Exception], None] | None): Handler for
                callback and signal-worker exceptions, or ``None`` to print
                tracebacks.
        """
        self._callbacks: dict[int, list[Callable[..., Any]]] = {}
        self._callbacks_previous: dict[int, list[Callable[..., Any]]] = {}
        self._server: StateServerCore = server

        self._workers_count: int = workers
        self._workers: list[threading.Thread] = []
        self._error_handler: Callable[[Exception], None] = error_handler or self._default_error_handler
        # Only the native server is referenced, so the finalizer does not keep the manager alive.
        weakref.finalize(self, server.signals_close)

    def start_manager(self) -> None:
        """Start callback worker threads unless they are already running."""
        if self._workers:
            return

        for i in range(self._workers_count):
            worker = threading.Thread(
                target=_run, args=(weakref.ref(self), self._server), daemon=True, name=f"signals_worker_{i}"
            )
            self._workers.append(worker)
            worker.start()

    def _invoke(self, callback: Callable[..., Any], *args: Any) -> None:
        """Invoke one callback without allowing it to stop the worker.

        Args:
            callback (Callable[..., Any]): Callback to invoke.
            *args (Any): Positional arguments forwarded to the callback.
        """
        try:
            callback(*args)
        except Exception as e:
            try:
                self._error_handler(e)
            except Exception:  # safety
                pass

    def _dispatch(self, value_id: int, arg: Any, has_previous: bool, previous: Any) -> None:
        """Invoke the callbacks registered for one claimed event.

        Args:
            value_id (int): Native protocol id of the state.
            arg (Any): Decoded value, or ``()`` for an empty signal.
            has_previous (bool): Whether ``previous`` carries a replaced value.
            previous (Any): Replaced value, when ``has_previous`` is true.
        """
        args = () if arg == () else (arg,)
        for callback in self._callbacks.get(value_id) or ():
            self._invoke(callback, *args)

        if has_previous:
            for callback in self._callbacks_previous.get(value_id) or ():
                self._invoke(callback, arg, previous)

    @staticmethod
    def _default_error_handler(_e: Exception) -> None:
        traceback.print_exc()

    def set_error_handler(self, error_handler: Callable[[Exception], None] | None) -> None:
        """Set the callback and worker error handler.

        Args:
            error_handler (Callable[[Exception], None] | None): Handler to use,
                or ``None`` to restore traceback printing.
        """
        self._error_handler = error_handler or self._default_error_handler

    def _register(self, value_id: int) -> None:
        """Update native registration for the callbacks on one state.

        Args:
            value_id (int): Native protocol id of the state.
        """
        has_previous = bool(self._callbacks_previous.get(value_id))
        has_any = has_previous or bool(self._callbacks.get(value_id))
        self._server.signal_register(value_id, has_any, has_previous)

    def add_callback(self, value_id: int, callback: Callable[..., Any]) -> None:
        """Add a callback to a signal.

        Args:
            value_id (int): Native protocol id of the signal.
            callback (Callable[..., Any]): Callback to register.
        """
        if value_id in self._callbacks:
            self._callbacks[value_id].append(callback)
        else:
            self._callbacks[value_id] = [callback]
        self._register(value_id)

    def add_callback_previous(self, value_id: int, callback: Callable[..., Any]) -> None:
        """Add a callback which also receives the previous value.

        Args:
            value_id (int): Native protocol id of the value.
            callback (Callable[..., Any]): Callback to register.
        """
        if value_id in self._callbacks_previous:
            self._callbacks_previous[value_id].append(callback)
        else:
            self._callbacks_previous[value_id] = [callback]
        self._register(value_id)

    def remove_callback(self, value_id: int, callback: Callable[..., Any]) -> None:
        """Remove a callback from a signal.

        Args:
            value_id (int): Native protocol id of the signal.
            callback (Callable[..., Any]): Callback to remove.
        """
        if value_id in self._callbacks:
            if callback in self._callbacks[value_id]:
                self._callbacks[value_id].remove(callback)
                self._register(value_id)

    def remove_callback_previous(self, value_id: int, callback: Callable[..., Any]) -> None:
        """Remove a previous-value callback from a value.

        Args:
            value_id (int): Native protocol id of the value.
            callback (Callable[..., Any]): Callback to remove.
        """
        if value_id in self._callbacks_previous:
            if callback in self._callbacks_previous[value_id]:
                self._callbacks_previous[value_id].remove(callback)
                self._register(value_id)

    def clear_callbacks(self, value_id: int) -> None:
        """Clear all callbacks from a signal.

        Args:
            value_id (int): Native protocol id of the signal.
        """
        if value_id in self._callbacks:
            self._callbacks[value_id].clear()
        if value_id in self._callbacks_previous:
            self._callbacks_previous[value_id].clear()
        self._register(value_id)
