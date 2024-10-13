from collections.abc import Callable
from types import ModuleType

from egui_pysync.structures import ErrorSignal, _MainStatesBase, _SignalsManager, _StatesBase, _ValueBase
from egui_pysync.typing import SteteServerCoreBase


def _initialize_states(obj, server: SteteServerCoreBase, signals_manager: _SignalsManager) -> None:
    for o in obj.__dict__.values():
        if isinstance(o, _ValueBase):
            o._initialize(server, signals_manager)
        elif isinstance(o, _StatesBase):
            _initialize_states(o, server, signals_manager)


class StateServer[T: _MainStatesBase]:
    """The main class for the SteteServer for UI."""

    def __init__(
        self,
        state_class: type[T],
        core_module: ModuleType,
        port: int,
        signals_workers: int = 3,
        error_handler: Callable[[Exception], None] | None = None,
        ip_addr: tuple[int, int, int, int] | None = None,
        handshake: list[int] | None = None,
    ) -> None:
        """Initialize the SteteServer."""
        core_server_class: type[SteteServerCoreBase] = getattr(core_module, "StateServerCore")
        self._server = core_server_class(port, ip_addr, handshake)
        self._signals_manager = _SignalsManager(self._server, signals_workers, error_handler)
        self._states: T = state_class(self._server.update)

        _initialize_states(self._states, self._server, self._signals_manager)
        self.error = ErrorSignal(self._signals_manager)

        self._signals_manager.close_registration()

    @property
    def states(self) -> T:
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
