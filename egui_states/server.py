# ruff: noqa: D107
from abc import ABC, abstractmethod
from collections.abc import Callable
from typing import Any

from egui_states._core import PyObjectType, StateServerCore
from egui_states.logging import LoggingSignal
from egui_states.signals import SignalsManager
from egui_states.structures import ISubStates, _SignalBase, _StaticBase

_ON_CONNECT_ID = 1
_ON_DISCONNECT_ID = 2
_CLIENT_MESSAGE_ID = 3


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


class StatesBase(ABC):
    """The root state class for the UI states."""

    def __init__(self, server: "StateServerBase") -> None:
        self._server = server

    def update_ui(self, dt: float | None = None) -> None:
        """Request the UI to update.

        Args:
            dt(float | None, optional): Delay time to next update, None means immediate. Defaults to None.
        """
        self._server.update(dt)

    def get_server(self) -> "StateServerBase":
        """Get the state server.

        Returns:
            StateServer: The state server.
        """
        return self._server

    @staticmethod
    @abstractmethod
    def _get_obj_types() -> list[PyObjectType]:
        pass


class StateServerBase[T: StatesBase]:
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
        self._states: T = state_class(self)

        _initialize(self._states, "root", self._server, self._signals_manager, self._states._get_obj_types())
        self._server.finalize()
        self.logging = LoggingSignal(self._signals_manager, self._server)
        self._on_connect: Callable[[str], Any] | None = None
        self._on_disconnect: Callable[[], Any] | None = None
        self._on_client_message: Callable[[str], Any] | None = None

        self._server.signal_set_to_queue(_ON_CONNECT_ID)
        self._server.signal_set_to_queue(_ON_DISCONNECT_ID)
        self._server.signal_set_to_queue(_CLIENT_MESSAGE_ID)

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

    def on_connect(self, func: Callable[[str], Any] | None) -> None:
        """Set the function to be called when a client connects.

        Args:
            func (Callable[[str], Any]): The function to be called when a client connects. It takes the client IP as an
                argument.
        """
        self._on_connect = func
        self._signals_manager.clear_callbacks(_ON_CONNECT_ID)
        if func is not None:
            self._signals_manager.add_callback(_ON_CONNECT_ID, func)

    def on_disconnect(self, func: Callable[[], Any] | None) -> None:
        """Set the function to be called when a client disconnects.

        Args:
            func (Callable[[], Any]): The function to be called when a client disconnects.
        """
        self._on_disconnect = func
        self._signals_manager.clear_callbacks(_ON_DISCONNECT_ID)
        if func is not None:
            self._signals_manager.add_callback(_ON_DISCONNECT_ID, func)

    def on_client_message(self, func: Callable[[str], Any] | None) -> None:
        """Set the function to be called when a client sends a message.

        Args:
            func (Callable[[str], Any]): The function to be called when a client sends a message. It takes the message
                string as an argument.
        """
        self._on_client_message = func
        self._signals_manager.clear_callbacks(_CLIENT_MESSAGE_ID)
        if func is not None:
            self._signals_manager.add_callback(_CLIENT_MESSAGE_ID, func)
