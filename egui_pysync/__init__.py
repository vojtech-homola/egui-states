from collections.abc import Callable

from egui_pysync.core import StateServer as _Server
from egui_pysync.states import States as _States
from egui_pysync.structures import _SignalsManager, ErrorSignal


class StateServer:
    """The main class for the SteteServer for UI."""

    def __init__(self, signals_workers: int = 3, error_handler: Callable[[Exception], None] | None = None) -> None:
        """Initialize the SteteServer."""
        self._server = _Server()
        self._signals_manager = _SignalsManager(self._server, signals_workers, error_handler)
        self._states = _States(self._server, self._signals_manager)

        self.error = ErrorSignal(self._signals_manager)

    @property
    def states(self) -> _States:
        """Get the state."""
        return self._states

    def update(self, duration: float | None = None) -> None:
        """Update the UI.

        Args:
            duration: The duration of the update.
        """
        self._server.update(duration)

    def start(self) -> None:
        """Start the state server."""
        self._server.start()

    def stop(self) -> None:
        """Stop the state server."""
        self._server.stop()

    def disconnect_client(self) -> None:
        """Disconnect actual client."""
        self._server.disconnect_client()

    def is_running(self) -> bool:
        """If state server is running."""
        return self._server.is_running()

    def is_connected(self) -> bool:
        """If client is connected to the state server."""
        return self._server.is_connected()

    def set_error_handler(self, error_handler: Callable[[Exception], None] | None) -> None:
        """Set the error handler.

        Function that will be called when an error occurs in the signals threads. By default, it prints the traceback.
        Be careful, if error is not handled, the thread will be stopped.

        Args:
            error_handler(Callable[[Exception], None] | None): The error handler function.
        """
        self._signals_manager.set_error_handler(error_handler)

    def check_workers(self) -> None:
        """Check all workers threads and restart them if they are stopped."""
        self._signals_manager.check_workers()
