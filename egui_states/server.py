# ruff: noqa: D107
"""Base classes used by generated Python state servers."""

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
    obj: object,
    parent: str,
    server: StateServerCore,
    signals_manager: SignalsManager,
    types: list[PyObjectType],
) -> None:
    """Attach generated state handles recursively to their native server ids.

    Args:
        obj (object): Generated root state or nested state group to initialize.
        parent (str): Fully qualified path of the parent state.
        server (StateServerCore): Native server that owns the states.
        signals_manager (SignalsManager):
            Dispatches callbacks for signal states.
        types (list[PyObjectType]): Protocol types referenced by generated
            states.
    """
    # Generated code escapes Python keywords (`global_`); paths use the original names.
    wire_names: dict[str, str] = getattr(type(obj), "__egui_names__", {})
    for name, o in obj.__dict__.items():
        full_name = f"{parent}.{wire_names.get(name, name)}"
        if isinstance(o, _StaticBase):
            o._initialize_base(server)
            if isinstance(o, _SignalBase):
                o._initialize_signal(signals_manager)
            o._initialize(full_name, types)
        elif isinstance(o, ISubStates):
            _initialize(o, full_name, server, signals_manager, types)


class StatesBase(ABC):
    """Generated root state tree owned by a Python server."""

    def __init__(self, server: "StateServerBase") -> None:
        self._server = server

    def update_ui(self, dt: float | None = None) -> None:
        """Request the UI to update.

        Args:
            dt (float | None, optional): Repaint delay in seconds, or ``None``
                for an immediate repaint.
        """
        self._server.update(dt)

    def get_server(self) -> "StateServerBase":
        """Return the state server that owns this tree.

        Returns:
            StateServerBase: The owning state server.
        """
        return self._server

    @staticmethod
    @abstractmethod
    def _get_obj_types() -> list[PyObjectType]:
        pass


class StateServerBase[T: StatesBase]:
    """Server that owns a generated state tree and its callback workers."""

    def __init__(
        self,
        state_class: type[T],
        signals_workers: int = 3,
        error_handler: Callable[[Exception], None] | None = None,
        version: int | None = None,
    ) -> None:
        """Initialize the state server.

        Args:
            state_class (type[T]): Generated root-state class.
            signals_workers (int, optional): Number of worker threads that
                invoke callbacks.
            error_handler (Callable[[Exception], None] | None, optional):
                Handler for exceptions raised while processing callbacks.
            version (int | None, optional): Application version required from
                the client.
        """
        self._server = StateServerCore(version)
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
        """The generated root state object.

        Returns:
            T: The generated root state object.
        """
        return self._states

    def update(self, duration: float | None = None) -> None:
        """Request an egui repaint on the connected client.

        Args:
            duration (float | None, optional): Repaint delay in seconds, or
                ``None`` for an immediate repaint.
        """
        self._server.update(duration)

    def start(
        self,
        port: int,
        ip_addr: tuple[int, int, int, int] | None = None,
        token: str | None = None,
    ) -> None:
        """Start the state server.

        After :meth:`stop`, the server can be started again with different
        connection settings. Calling this method while the server is already
        running succeeds without changing its active settings.

        Args:
            port (int): TCP port on which the WebSocket server listens.
            ip_addr (tuple[int, int, int, int] | None, optional): IPv4 address
                to bind, or ``None`` to bind all interfaces.
            token (str | None, optional): Authentication token required from
                the client.
        """
        self._server.start(port, ip_addr, token)
        self._signals_manager.start_manager()

    def stop(self) -> None:
        """Stop the state server."""
        self._server.stop()

    def disconnect_client(self) -> None:
        """Disconnect the current client while leaving the server running."""
        self._server.disconnect_client()

    def is_running(self) -> bool:
        """Return whether the server is listening.

        Returns:
            bool: Whether the server is listening.
        """
        return self._server.is_running()

    def is_connected(self) -> bool:
        """Return whether a client is connected.

        Returns:
            bool: Whether a client is connected.
        """
        return self._server.is_connected()

    def set_error_handler(self, error_handler: Callable[[Exception], None] | None) -> None:
        """Set the handler for callback and signal-worker errors.

        The default handler prints a traceback. Exceptions raised by the
        handler while processing a callback error are ignored; the handler
        should return normally so errors from the native signal loop do not
        terminate that worker.

        Args:
            error_handler (Callable[[Exception], None] | None): Handler to
                install, or ``None`` to restore the default.
        """
        self._signals_manager.set_error_handler(error_handler)

    def on_connect(self, func: Callable[[str], Any] | None) -> None:
        """Replace the callback invoked when a client connects.

        Args:
            func (Callable[[str], Any] | None): Callback receiving the client
                address, or ``None`` to unregister the current callback.
        """
        self._on_connect = func
        self._signals_manager.clear_callbacks(_ON_CONNECT_ID)
        if func is not None:
            self._signals_manager.add_callback(_ON_CONNECT_ID, func)

    def on_disconnect(self, func: Callable[[], Any] | None) -> None:
        """Replace the callback invoked when a client disconnects.

        Args:
            func (Callable[[], Any] | None): Callback to invoke, or ``None`` to
                unregister the current callback.
        """
        self._on_disconnect = func
        self._signals_manager.clear_callbacks(_ON_DISCONNECT_ID)
        if func is not None:
            self._signals_manager.add_callback(_ON_DISCONNECT_ID, func)

    def on_client_message(self, func: Callable[[str], Any] | None) -> None:
        """Replace the callback for diagnostic messages from the client.

        Args:
            func (Callable[[str], Any] | None): Callback receiving the
                diagnostic message, or ``None`` to unregister the current
                callback.
        """
        self._on_client_message = func
        self._signals_manager.clear_callbacks(_CLIENT_MESSAGE_ID)
        if func is not None:
            self._signals_manager.add_callback(_CLIENT_MESSAGE_ID, func)
