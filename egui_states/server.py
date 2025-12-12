from collections.abc import Callable

from egui_states._core import PyObjectType, StateServerCore
from egui_states.logging import LoggingSignal
from egui_states.signals import SignalsManager
from egui_states.structures import StatesBase, ISubStates, _SignalBase, _StaticBase


def _initialize(
    obj, parent: str, server: StateServerCore, signals_manager: SignalsManager, types: list[PyObjectType]
) -> None:
    for name, o in obj.__dict__.items():
        full_name = f"{parent}.{name}"
        if isinstance(o, _StaticBase):
            o._initialize_base(server)
            if isinstance(o, _SignalBase):
                o._initialize_signal(signals_manager)
            o._initialize(full_name, types)
        elif isinstance(o, ISubStates):
            _initialize(o, full_name, server, signals_manager, types)


class StateServer[T: StatesBase]:
    """The main class for the SteteServer for UI."""

    def __init__(
        self,
        state_class: type[T],
        port: int,
        signals_workers: int = 3,
        error_handler: Callable[[Exception], None] | None = None,
        ip_addr: tuple[int, int, int, int] | None = None,
        handshake: list[int] | None = None,
    ) -> None:
        """Initialize the SteteServer.

        Args:
            state_class (RoorState): The class representing the UI states.
            port (int): The port to run the server on.
            signals_workers (int): The number of worker threads for signal handling.
            error_handler (Callable[[Exception], None] | None): The error handler function.
            ip_addr (tuple[int, int, int, int] | None): The IP address to bind the server to.
            handshake (list[int] | None): The handshake bytes for client connection.
        """
        self._server = StateServerCore(port, ip_addr, handshake)
        self._signals_manager = SignalsManager(self._server, signals_workers, error_handler)
        self._states: T = state_class(self._server.update)

        _initialize(self._states, "root", self._server, self._signals_manager, self._states._get_obj_types())
        self._server.finalize()
        self.logging = LoggingSignal(self._signals_manager, self._server)

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
        self._signals_manager.start_manager()
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
