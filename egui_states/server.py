# ruff: noqa: D107
"""Base classes used by generated Python state servers."""

from abc import ABC, abstractmethod
from collections.abc import Callable, Sequence
from enum import Enum
from os import PathLike
from pathlib import Path
from typing import Any

from egui_states._core import PyObjectType, StateServerCore, _ConnectRequest
from egui_states.logging import LoggingSignal
from egui_states.signals import SignalsManager
from egui_states.structures import ISubStates, _CallbackBase, _StaticBase
from egui_states.transfer import TransferProfile

_ON_CONNECT_ID = 1
_ON_DISCONNECT_ID = 2
_CLIENT_MESSAGE_ID = 3
_ON_CONNECT_REQUEST_ID = 4


class ClientPolicy(Enum):
    """What the server does with a client that connects while another is connected.

    The server serves one client at a time.
    """

    #: The new client takes over the connection. The connected client is
    #: closed and learns that it was replaced.
    Replace = 0
    #: The connected client keeps the connection. The new client is refused
    #: and learns that the server is busy.
    Reject = 1


class ConnectRequest:
    """A client that passed the handshake and asks to connect.

    The callback registered with :meth:`StateServerBase.on_connect_request`
    receives it and decides whether the client may connect.
    """

    __slots__ = ("_request",)

    def __init__(self, request: _ConnectRequest) -> None:
        self._request = request

    @property
    def address(self) -> str:
        """The client's address, as ``ip:port``."""
        return self._request.address()

    def is_pending(self) -> bool:
        """Return whether the server still waits for the answer.

        It turns ``False`` once the request was answered, when the client
        closes its connection, when ``connect_request_timeout`` passes, or
        when the server stops. An answer given after that is ignored, so a
        callback that waits, for example for the user of the connected
        client, can check this to stop waiting early.

        Returns:
            bool: Whether the answer is still awaited.
        """
        return self._request.is_pending()

    def __repr__(self) -> str:
        """Return the request's address and whether it is pending.

        Returns:
            str: Readable description of the request.
        """
        return f"ConnectRequest(address={self.address!r}, pending={self.is_pending()})"


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
            if isinstance(o, _CallbackBase):
                o._initialize_signal(signals_manager)
            o._initialize(full_name, types)
        elif isinstance(o, ISubStates):
            _initialize(o, full_name, server, signals_manager, types)


class StatesBase(ABC):
    """Generated root state tree owned by a Python server."""

    def __init__(self, server: "StateServerBase") -> None:
        self._server = server

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
        heartbeat_timeout: float | None = 20.0,
        web_path: str | PathLike[str] | None = None,
        allowed_origins: Sequence[str] = (),
        transfer_profile: TransferProfile = TransferProfile.Balanced,
        client_policy: ClientPolicy = ClientPolicy.Replace,
        connect_request_timeout: float = 5.0,
        tls: bool = False,
        tls_cert: str | PathLike[str] | None = None,
        tls_key: str | PathLike[str] | None = None,
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
            heartbeat_timeout (float | None, optional): Seconds either side
                may hear nothing from the other before it treats the
                connection as lost. The server pings the client every quarter
                of it and sends a keep-alive message when it has nothing else
                to send, so this detects a peer that disappeared without
                closing the connection. Keep it above the time the largest
                transfer takes over the slowest expected network. ``None``
                disables the heartbeat on both sides, for example while
                debugging with breakpoints.
            web_path (str | PathLike[str] | None, optional): Directory of a
                static web page, such as a built WASM client, that
                :meth:`start_web` serves on the server port. A relative path
                is resolved against the current directory now; the directory
                is read only by :meth:`start_web`. ``None`` means no page can
                be served.
            allowed_origins (Sequence[str], optional): Origins of other web
                pages that may connect, as ``scheme://host[:port]`` patterns.
                Browsers let any page open a WebSocket to the server but state
                the page's origin, which the page cannot change. By default
                only pages served by the server itself (with
                :meth:`start_web` or through a proxy that sets
                ``X-Forwarded-Host``, such as ``trunk serve``) on an IP
                address or local network name may connect: a name without a
                dot (``localhost``, ``pc-a``) or ending in ``.local``,
                ``.home.arpa``, ``.internal``, or ``.localhost``. ``*``
                matches any scheme, any part of the host
                (``https://*.example.com``), or any port
                (``http://myserver.corp.example:*``, which allows the page
                opened by that name). Without a port, the scheme's default
                port is meant. An entry of just ``*`` allows every page,
                including local files; any site open in the browser can then
                connect, so set a token as well. Native clients send no origin
                and are not affected.
            transfer_profile (TransferProfile, optional): How file transfers
                share the connection with state synchronization; the client
                uses the server's profile. ``Responsive`` keeps at most 64 KiB
                of file data ahead of a state change, for slow links (VPN,
                mobile, under about 20 Mbit/s), but moves at most 64 KiB per
                round trip. ``Balanced`` keeps at most 256 KiB ahead and is
                close to full speed from 100 Mbit/s up. ``Throughput`` gives
                the fastest transfers, for a client on the same machine or
                when transfers matter more than the UI; a state change may
                wait behind up to 1 MiB of file data.
            client_policy (ClientPolicy, optional): What the server does with
                a client that connects while another is connected:
                ``Replace`` lets the new client take over, ``Reject`` keeps
                the connected client and refuses the new one as busy. A
                callback registered with :meth:`on_connect_request` decides
                instead.
            connect_request_timeout (float, optional): Seconds the callback
                registered with :meth:`on_connect_request` may take to decide
                on a client, counted from the client's handshake. A client
                still waiting when it passes is refused, which is logged as
                an error. The client learns this time and waits as long.
            tls (bool, optional): Whether clients connect with TLS, which
                encrypts the connection. The server then speaks only TLS on
                its port: ``wss://`` WebSockets, and ``https://`` for the web
                page and browser downloads. A plain HTTP request from a
                browser is redirected to ``https://``; other plain
                connections, such as a native client without TLS, are refused.
                Without ``tls_cert``, the server generates a self-signed
                certificate now. It exists only in memory, so every new
                server presents a new one; stopping and starting the same
                server keeps it. Native clients pin it by its fingerprint
                (see :meth:`tls_fingerprint`) or accept any certificate;
                browsers show a warning before the page loads.
            tls_cert (str | PathLike[str] | None, optional): PEM file with the
                certificate to present instead of a generated one, followed by
                any intermediate certificates. Requires ``tls`` and
                ``tls_key``. Clients that trust the certificate, or the
                authority that issued it, connect without a warning, provided
                they reach the server under a name or address it lists.
            tls_key (str | PathLike[str] | None, optional): PEM file with the
                unencrypted private key of ``tls_cert``.

        Raises:
            ValueError: If ``heartbeat_timeout`` is not positive,
                ``web_path`` cannot be made absolute (for example because it
                is empty), an allowed origin is malformed,
                ``transfer_profile`` is not a :class:`TransferProfile`,
                ``client_policy`` is not a :class:`ClientPolicy`,
                ``connect_request_timeout`` is not positive, only one of
                ``tls_cert`` and ``tls_key`` is given or either without
                ``tls``, a TLS file holds no valid PEM certificate or
                unencrypted key, or the key does not belong to the
                certificate.
            OSError: If ``tls_cert`` or ``tls_key`` cannot be read, such as
                ``FileNotFoundError`` for a missing file.
        """
        self._server = StateServerCore(
            version,
            heartbeat_timeout,
            web_path,
            list(allowed_origins),
            # The native core takes the profile's and the policy's numbers.
            TransferProfile(transfer_profile).value,
            ClientPolicy(client_policy).value,
            connect_request_timeout,
            tls=tls,
            tls_cert=tls_cert,
            tls_key=tls_key,
        )
        self._signals_manager = SignalsManager(self._server, signals_workers, error_handler)
        self._states: T = state_class(self)

        _initialize(self._states, "root", self._server, self._signals_manager, self._states._get_obj_types())
        self._server.finalize()
        self.logging = LoggingSignal(self._signals_manager)
        self._on_connect: Callable[[str], Any] | None = None
        self._on_disconnect: Callable[[], Any] | None = None
        self._on_client_message: Callable[[str], Any] | None = None
        self._on_connect_request: Callable[[ConnectRequest], bool] | None = None

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

        Calling this method while the server is running with the same settings
        succeeds without effect; port 0 also matches the port it was assigned
        (see :meth:`address`). To change the address or token, call
        :meth:`stop` first.

        Args:
            port (int): TCP port on which the WebSocket server listens.
            ip_addr (tuple[int, int, int, int] | None, optional): IPv4 address
                to bind, or ``None`` to bind only the loopback address
                ``127.0.0.1``. Pass ``(0, 0, 0, 0)`` to accept clients from
                other machines.
            token (str | None, optional): Authentication token required from
                the client.

        Raises:
            RuntimeError: If the server is already running with a different
                address or token, or the socket cannot be bound.
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

    def address(self) -> tuple[tuple[int, int, int, int], int] | None:
        """Return the address the server listens on.

        After starting on port 0, this reports the port the system assigned.

        Returns:
            tuple[tuple[int, int, int, int], int] | None: IPv4 address and
            port, or ``None`` when the server is not running.
        """
        return self._server.address()

    def tls_fingerprint(self) -> str | None:
        """Return the fingerprint of the server's TLS certificate.

        The fingerprint is the SHA-256 hash of the certificate, generated or
        supplied, and is known as soon as the server is created. Pass it to
        native clients (``Tls::Fingerprint`` in Rust, for example as a
        command-line argument) so they connect to this server only, and
        compare it with the SHA-256 fingerprint a browser shows for the
        certificate before accepting it.

        Returns:
            str | None: 64 lowercase hexadecimal digits, or ``None`` when TLS
            is off.
        """
        return self._server.tls_fingerprint()

    def start_web(self) -> None:
        """Start serving the static web page on the state server port.

        The page is read from the ``web_path`` the server was created with.
        This lets a browser load a WASM client from the same port it then
        connects to. WebSocket upgrade requests reach the state server as
        before; other HTTP ``GET`` and ``HEAD`` requests are answered from the
        page, with ``/`` and paths ending in ``/`` mapped to ``index.html``.

        Every file below the directory is read into memory during this call,
        so later changes on disk are not served until it is called again;
        calling it while the page is served reloads it. Serving may be started
        before :meth:`start`, applies to connections accepted afterwards, and
        stays on across :meth:`stop` and restarts until :meth:`stop_web`. A
        page load that overlaps a reload may combine files of both versions;
        reloading the page in the browser resolves it.

        Files and directories whose names start with ``.`` (such as ``.git``
        or ``.env``) are skipped. Symlinks are followed, except a link back to
        a directory that contains it.

        Raises:
            RuntimeError: If the server was created without ``web_path``.
            OSError: If the path or a file below it cannot be read, such as
                ``FileNotFoundError`` for a missing path. A page already being
                served is kept.
            ValueError: If the path is not a directory, the directory does not
                contain ``index.html``, or its files exceed 128 MiB. A page
                already being served is kept.
        """
        self._server.start_web()

    def stop_web(self) -> None:
        """Stop serving the static web page.

        The port then accepts only WebSocket connections, as if
        :meth:`start_web` had never been called. Requests already being
        answered complete; the connected client is not affected. Calling it
        while no page is served has no effect.
        """
        self._server.stop_web()

    def is_web_running(self) -> bool:
        """Return whether the static web page is being served.

        This is true from a successful :meth:`start_web` until
        :meth:`stop_web`, also while the server itself is stopped; browsers
        reach the page only while :meth:`is_running`.

        Returns:
            bool: True while web serving is on.
        """
        return self._server.is_web_running()

    def web_path(self) -> Path | None:
        """Return the absolute directory of the configured web page.

        The path is returned whether or not the page is being served.

        Returns:
            Path | None: Page directory, or ``None`` when the server was
            created without ``web_path``.
        """
        path = self._server.web_path()
        return None if path is None else Path(path)

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

    def on_connect_request(self, func: Callable[[ConnectRequest], bool] | None) -> None:
        """Replace the callback that decides whether a client may connect.

        The server asks it for every client that passed the handshake, whether
        or not another client is connected, instead of applying
        ``client_policy``. Returning ``True`` lets the client in, replacing the
        connected one; ``False`` refuses it, and the client learns that it
        was refused. A callback that raises refuses the client and reports
        the exception to the error handler.

        The callback runs on a callback worker and may block, for example to
        ask the user of the connected client; it then occupies that worker,
        so keep ``signals_workers`` at two or more for the user's answer to
        arrive. Requests are decided one at a time; meanwhile the client waits
        and the connected client is unaffected. A request not answered within
        ``connect_request_timeout`` is refused, which is logged as an error,
        and its answer ignored; check :meth:`ConnectRequest.is_pending` to
        stop waiting early. While the callback blocks, the requests behind it
        wait and may run out of time without being asked about.

        Args:
            func (Callable[[ConnectRequest], bool] | None): Callback receiving
                the request and returning whether to accept the client, or
                ``None`` to unregister it and return the decision to
                ``client_policy``.
        """
        # The dispatcher stays registered while a callback is swapped: without
        # it, a client connecting in between would be decided by the policy.
        registered = self._on_connect_request is not None
        self._on_connect_request = func
        if func is None:
            self._signals_manager.clear_callbacks(_ON_CONNECT_REQUEST_ID)
        elif not registered:
            self._signals_manager.add_callback(_ON_CONNECT_REQUEST_ID, self._decide_connect_request)

    def _decide_connect_request(self) -> None:
        """Ask the connect-request callback about the pending request."""
        request = self._server.connect_request()
        func = self._on_connect_request
        # A request that ended while it waited for this worker, because its
        # client left or its time ran out, is not asked about.
        if request is None or not request.is_pending():
            return
        accept = False
        try:
            accept = func is not None and bool(func(ConnectRequest(request)))
        finally:
            # An exception refuses the client; the worker then reports it.
            request.answer(accept)

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
