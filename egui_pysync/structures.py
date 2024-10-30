import threading
import traceback
from abc import ABC, abstractmethod
from collections.abc import Buffer, Callable

from egui_pysync.typing import SteteServerCoreBase


class _Counter:
    def __init__(self) -> None:
        self._counter = 9  # first 10 values are reserved for system signals

    def get_id(self) -> int:
        self._counter += 1
        return self._counter


class _StatesBase:
    pass


class _MainStatesBase(_StatesBase, ABC):
    @abstractmethod
    def __init__(self, update: Callable[[float | None], None]) -> None:
        pass


class _SignalsManager:
    def __init__(
        self,
        server: SteteServerCoreBase,
        workers: int,
        error_handler: Callable[[Exception], None] | None,
    ):
        self._callbacks: dict[int, list[Callable]] = {}
        self._args_parsers: dict[int, Callable] = {}
        self._server = server

        self._workers_count = workers
        self._workers: list[threading.Thread] = []
        self._error_handler = error_handler or self._default_error_handler

    def register_value(self, value_id: int) -> None:
        self._callbacks[value_id] = []

    def close_registration(self) -> None:
        for i in range(self._workers_count):
            worker = threading.Thread(target=self._run, args=(i,), daemon=True, name=f"signals_worker_{i}")
            self._workers.append(worker)
            worker.start()

    def check_workers(self) -> None:
        for i, worker in enumerate(self._workers):
            if not worker.is_alive():
                self._workers[i] = threading.Thread(
                    target=self._run, args=(i,), daemon=True, name=f"signals_worker_{i}"
                )
                self._workers[i].start()

    def _run(self, thread_id) -> None:
        while True:
            ind, args = self._server.value_get_signal(thread_id)
            if ind in self._callbacks:
                if ind in self._args_parsers:
                    for callback in self._callbacks[ind]:
                        try:
                            callback(self._args_parsers[ind](*args))
                        except Exception as e:
                            self._error_handler(e)
                else:
                    for callback in self._callbacks[ind]:
                        try:
                            callback(*args)
                        except Exception as e:
                            self._error_handler(e)
            else:
                error = IndexError(f"Signal with index {ind} not found.")
                self._error_handler(error)

    @staticmethod
    def _default_error_handler(_e: Exception) -> None:
        traceback.print_exc()

    def set_error_handler(self, error_handler: Callable[[Exception], None] | None) -> None:
        self._error_handler = error_handler or self._default_error_handler

    def add_callback(self, value_id: int, callback: Callable, args_parser: Callable | None = None) -> None:
        if value_id in self._callbacks:
            self._callbacks[value_id].append(callback)
            if args_parser:
                self._args_parsers[value_id] = args_parser
            self._server.value_set_register(value_id, True)

    def remove_callback(self, value_id: int, callback: Callable) -> None:
        if value_id in self._callbacks and callback in self._callbacks[value_id]:
            self._callbacks[value_id].remove(callback)
            if not self._callbacks[value_id]:
                self._server.value_set_register(value_id, False)

    def clear_callbacks(self, value_id: int) -> None:
        if value_id in self._callbacks:
            self._callbacks[value_id].clear()
            self._server.value_set_register(value_id, False)


class ErrorSignal:
    """Error signal for processing errors from the state server."""

    def __init__(self, siganls_manager: _SignalsManager):
        """Initialize the ErrorSignal."""
        self._value_id = 0
        self._signals_manager = siganls_manager
        self._signals_manager.register_value(0)

    def connect(self, callback: Callable[[str], None]) -> None:
        """Connect a callback to the value.

        Args:
            callback(Callable[[str], None]): The callback to connect.
        """
        self._signals_manager.add_callback(self._value_id, callback)

    def disconnect(self, callback: Callable[[str], None]) -> None:
        """Disconnect a callback from the value.

        Args:
            callback(Callable[[T], None]): The callback to disconnect.
        """
        self._signals_manager.remove_callback(self._value_id, callback)

    def disconnect_all(self) -> None:
        """Disconnect all callbacks from the value."""
        self._signals_manager.clear_callbacks(self._value_id)


class _ValueBase:
    _has_signal: bool = True

    _server: SteteServerCoreBase
    _signals_manager: _SignalsManager

    def __init__(self, counter: _Counter):
        self._value_id = counter.get_id()

    def _initialize(self, server: SteteServerCoreBase, signals_manager: _SignalsManager):
        self._server = server
        if self._has_signal:
            self._signals_manager = signals_manager
            signals_manager.register_value(self._value_id)


class Value[T](_ValueBase):
    """General UI value of type T."""

    def set(self, value: T, set_signal: bool = True, update: bool = False) -> None:
        """Set the value of the UI element.

        Args:
            value(T): The value to set.
            set_signal(bool, optional): Whether to set the signal. Defaults to True.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.value_set(self._value_id, value, set_signal, update)

    def get(self) -> T:
        """Get the value of the UI element.

        Returns:
            T: The value of the UI element.
        """
        return self._server.value_get(self._value_id)

    def connect(self, callback: Callable[[T], None]) -> None:
        """Connect a callback to the value.

        Args:
            callback(Callable[[T], None]): The callback to connect.
        """
        self._signals_manager.add_callback(self._value_id, callback)

    def disconnect(self, callback: Callable[[T], None]) -> None:
        """Disconnect a callback from the value.

        Args:
            callback(Callable[[T], None]): The callback to disconnect.
        """
        self._signals_manager.remove_callback(self._value_id, callback)

    def disconnect_all(self) -> None:
        """Disconnect all callbacks from the value."""
        self._signals_manager.clear_callbacks(self._value_id)


class ValueStatic[T](_ValueBase):
    """Numeric static UI value of type T. Static means that the value is not updated in the UI."""

    _has_signal = False

    def set(self, value: T, update: bool = False) -> None:
        """Set the static value of the UI.

        Args:
            value(T): The value to set.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.static_set(self._value_id, value, update)

    def get(self) -> T:
        """Get the static value of the UI.

        Returns:
            T: The static value.
        """
        return self._server.static_get(self._value_id)


class ValueEnum[T](_ValueBase):
    """Enum UI value of type T."""

    def __init__(self, counter: _Counter, enum_type: type[T]):  # noqa: D107
        super().__init__(counter)
        self._enum_type = enum_type

    def set(self, value: T, set_signal: bool = True, update: bool = False) -> None:
        """Set the value of the UI element.

        Args:
            value(T): The value to set.
            set_signal(bool, optional): Whether to set the signal. Defaults to True.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.value_set(self._value_id, value.value, set_signal, update)  # type: ignore

    def get(self) -> T:
        """Get the value of the UI element.

        Returns:
            T: The value of the UI element.
        """
        str_value: int = self._server.value_get(self._value_id)
        return self._enum_type(str_value)  # type: ignore

    def connect(self, callback: Callable[[T], None]) -> None:
        """Connect a callback to the value.

        Args:
            callback(Callable[[T], None]): The callback to connect.
        """
        self._signals_manager.add_callback(self._value_id, callback, args_parser=self._enum_type)

    def disconnect(self, callback: Callable[[T], None]) -> None:
        """Disconnect a callback from the value.

        Args:
            callback(Callable[[T], None]): The callback to disconnect.
        """
        self._signals_manager.remove_callback(self._value_id, callback)

    def disconnect_all(self) -> None:
        """Disconnect all callbacks from the value."""
        self._signals_manager.clear_callbacks(self._value_id)


class Signal[T](_ValueBase):
    """Signal from UI."""

    def connect(self, callback: Callable[[T], None]) -> None:
        """Connect a callback to the signal.

        Args:
            callback(Callable[[], None]): The callback to connect.
        """
        self._signals_manager.add_callback(self._value_id, callback)

    def disconnect(self, callback: Callable[[T], None]) -> None:
        """Disconnect a callback from the value.

        Args:
            callback(Callable[[], None]): The callback to disconnect.
        """
        self._signals_manager.remove_callback(self._value_id, callback)

    def disconnect_all(self) -> None:
        """Disconnect all callbacks from the signal."""
        self._signals_manager.clear_callbacks(self._value_id)


class SignalEmpty(_ValueBase):
    """Empty Signal from UI."""

    def connect(self, callback: Callable[[], None]) -> None:
        """Connect a callback to the signal.

        Args:
            callback(Callable[[], None]): The callback to connect.
        """
        self._signals_manager.add_callback(self._value_id, callback)

    def disconnect(self, callback: Callable[[], None]) -> None:
        """Disconnect a callback from the value.

        Args:
            callback(Callable[[], None]): The callback to disconnect.
        """
        self._signals_manager.remove_callback(self._value_id, callback)

    def disconnect_all(self) -> None:
        """Disconnect all callbacks from the signal."""
        self._signals_manager.clear_callbacks(self._value_id)


class ValueImage(_ValueBase):
    """Image UI element."""

    _has_signal = False

    def set_image(
        self,
        image: Buffer,
        rect: list[int] | None = None,
        update: bool = False,
    ) -> None:
        """Set the image in the UI image.

        Args:
            image(Buffer): The image to set.
            rect(list[int], optional): The rectangle [y, x, height, width]. Defaults to None.
            update(bool, optional): Whether to update the UI. Defaults to True.
        """
        self._server.image_set(self._value_id, image, update, rect)

    def set_histogram(self, histogram: Buffer | None = None, update: bool = False) -> None:
        """Set the histogram in the UI image.

        Args:
            histogram(Buffer, optional): The histogram numpy array of float32 normalized to 1. Defaults to None.
            update(bool, optional): Whether to update the UI. Defaults to True.
        """
        self._server.histogram_set(self._value_id, update, histogram)


class ValueDict[K, V](_ValueBase):
    """Dict UI element."""

    _has_signal = False

    def set(self, value: dict[K, V], update: bool = False) -> None:
        """Set the dict in the UI dict.

        Args:
            value(dict[K, V]): The dict to set.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.dict_set(self._value_id, value, update)

    def get(self) -> dict[K, V]:
        """Get the dict in the UI dict.

        Returns:
            dict[K, V]: The dict in the UI dict.
        """
        return self._server.dict_get(self._value_id)

    def set_item(self, key: K, value: V, update: bool = False) -> None:
        """Set the item in the UI dict.

        Args:
            key(K): The key of the item.
            value(V): The value of the item.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.dict_item_set(self._value_id, key, value, update)

    def get_item(self, key: K) -> V:
        """Get the item in the UI dict.

        Args:
            key(K): The key of the item.

        Returns:
            V: The value of the item.
        """
        return self._server.dict_item_get(self._value_id, key)

    def remove_item(self, key: K, update: bool = False) -> None:
        """Remove the item from the UI dict.

        Args:
            key(K): The key of the item.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.dict_item_del(self._value_id, key, update)

    def __getitem__(self, key: K) -> V:
        """Get the item in the UI dict."""
        return self.get_item(key)

    def __setitem__(self, key: K, value: V) -> None:
        """Set the item in the UI dict."""
        self.set_item(key, value, update=False)

    def __delitem__(self, key: K) -> None:
        """Remove the item from the UI dict."""
        self.remove_item(key, update=False)


class ValueList[T](_ValueBase):
    """List UI element."""

    _has_signal = False

    def set(self, value: list[T], update: bool = False) -> None:
        """Set the list in the UI list.

        Args:
            value(list[T]): The list to set.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.list_set(self._value_id, value, update)

    def get(self) -> list[T]:
        """Get the list in the UI list.

        Returns:
            list[T]: The list in the UI list.
        """
        return self._server.list_get(self._value_id)

    def set_item(self, idx: int, value: T, update: bool = False) -> None:
        """Set the item in the UI list.

        Args:
            idx(int): The index of the item.
            value(T): The value of the item.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.list_item_set(self._value_id, idx, value, update)

    def get_item(self, idx: int) -> T:
        """Get the item in the UI list.

        Args:
            idx(int): The index of the item.

        Returns:
            T: The value of the item.
        """
        return self._server.list_item_get(self._value_id, idx)

    def remove_item(self, idx: int, update: bool = False) -> None:
        """Remove the item from the UI list.

        Args:
            idx(int): The index of the item.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.list_item_del(self._value_id, idx, update)

    def add_item(self, value: T, update: bool = False) -> None:
        """Add the item to the UI list.

        Args:
            value(T): The value of the item.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.list_item_add(self._value_id, value, update)

    def __getitem__(self, idx: int) -> T:
        """Get the item in the UI list."""
        return self.get_item(idx)

    def __setitem__(self, idx: int, value: T) -> None:
        """Set the item in the UI list."""
        self.set_item(idx, value, update=False)


class ValueGraph(_ValueBase):
    """Graph UI element."""

    _has_signal = False

    def set(self, graph: Buffer, update: bool = False) -> None:
        """Set the graph in the UI graph.

        Args:
            graph(Buffer): The graph to set.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.set_graph(self._value_id, graph, update)

    def add_points(self, points: Buffer, update: bool = False) -> None:
        """Add the points to the UI graph.

        Args:
            points(Buffer): The points to add.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.add_graph_points(self._value_id, points, update)

    def clear(self, update: bool = False) -> None:
        """Clear the UI graph.

        Args:
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.clear_graph(self._value_id, update)
