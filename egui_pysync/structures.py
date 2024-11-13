import threading
import traceback
from abc import ABC, abstractmethod
from collections.abc import Buffer, Callable
from typing import Any

import numpy as np

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


type ArgParser = Callable[[Any], tuple[Any, ...]]


class _SignalsManager:
    def __init__(
        self,
        server: SteteServerCoreBase,
        workers: int,
        error_handler: Callable[[Exception], None] | None,
    ):
        self._callbacks: dict[int, tuple[list[Callable], ArgParser | None]] = {}
        self._server = server

        self._workers_count = workers
        self._workers: list[threading.Thread] = []
        self._error_handler = error_handler or self._default_error_handler

    def register_value(self, value_id: int, arg_parser: ArgParser | None = None) -> None:
        self._callbacks[value_id] = ([], arg_parser)

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
            ind, arg = self._server.value_get_signal(thread_id)
            callbacks, arg_parser = self._callbacks.get(ind, (None, None))
            if callbacks:
                if arg_parser is None:
                    for callback in callbacks:
                        try:
                            callback(arg)
                        except Exception as e:
                            self._error_handler(e)
                else:
                    for callback in callbacks:
                        try:
                            callback(*arg_parser(arg))
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

    def add_callback(self, value_id: int, callback: Callable) -> None:
        if value_id in self._callbacks:
            self._callbacks[value_id][0].append(callback)
            self._server.value_set_register(value_id, True)
        else:
            raise RuntimeError(f"Signal with index {value_id} not found.")

    def remove_callback(self, value_id: int, callback: Callable) -> None:
        if value_id in self._callbacks:
            if callback in self._callbacks[value_id]:
                self._callbacks[value_id][0].remove(callback)
                if not self._callbacks[value_id]:
                    self._server.value_set_register(value_id, False)
        else:
            raise RuntimeError(f"Signal with index {value_id} not found.")

    def clear_callbacks(self, value_id: int) -> None:
        if value_id in self._callbacks:
            self._callbacks[value_id][0].clear()
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


class _StaticBase:
    _server: SteteServerCoreBase

    def __init__(self, counter: _Counter):
        self._value_id = counter.get_id()

    def _initialize(self, server: SteteServerCoreBase):
        self._server = server


class _ValueBase(_StaticBase):
    _signals_manager: _SignalsManager

    def _initialize(self, server: SteteServerCoreBase, signals_manager: _SignalsManager):
        self._server = server
        self._signals_manager = signals_manager
        if hasattr(self, "_arg_parser"):
            arg_parser = getattr(self, "_arg_parser")
            self._signals_manager.register_value(self._value_id, arg_parser=arg_parser)
        else:
            signals_manager.register_value(self._value_id)


class Value[T](_ValueBase):
    """General UI value of type T."""

    def set(self, value: T, set_signal: bool = False, update: bool = False) -> None:
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


class ValueStatic[T](_StaticBase):
    """Numeric static UI value of type T. Static means that the value is not updated in the UI."""

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

    def set(self, value: T, set_signal: bool = False, update: bool = False) -> None:
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

    def _arg_parser(self, arg: int):
        return (self._enum_type(arg),)  # type: ignore


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

    @staticmethod
    def _arg_parser(_: None):
        return ()


class ValueImage(_StaticBase):
    """Image UI element."""

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


class ValueDict[K, V](_StaticBase):
    """Dict UI element."""

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


class ValueList[T](_StaticBase):
    """List UI element."""

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


class _GraphBase:
    def __init__(self, value_id: int, idx: int, server: SteteServerCoreBase):
        self._value_id = value_id
        self._idx = idx
        self._server = server

        self._deleted = False

    @property
    def idx(self) -> int:
        return self._idx

    @property
    def active(self) -> bool:
        return not self._deleted

    def len(self) -> int:
        """Get the length of the graph.

        Returns:
            int: The length of the graph.
        """
        self._check()
        return self._server.graphs_len(self._value_id, self._idx)

    def _kill(self):
        self._deleted = True
        self._server.graphs_remove(self._value_id, self._idx, update=False)

    def _check(self):
        if self._deleted:
            raise RuntimeError("Graph was deleted. You have to create a new one.")

    def __len__(self) -> int:
        return self.len()


class Graph(_GraphBase):
    """Graph UI element."""

    def add_points(self, points: Buffer, update: bool = False) -> None:
        """Add the points to the graph.

        Args:
            points(Buffer): The points to add. Has to implement the buffer protocol.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._check()
        self._server.graphs_add_points(self._value_id, self._idx, points, update, range=None)

    def set(self, graph: Buffer, update: bool = False) -> None:
        """Set the graph to the UI graphs.

        Args:
            graph(Buffer): The graph to set. Has to implement the buffer protocol.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._check()
        self._server.graphs_set(self._value_id, self._idx, graph, update, range=None)

    def get(self) -> np.ndarray:
        """Get the graph from the UI graphs.

        Returns:
            np.ndarray: The graph.
        """
        data, shape, range = self._server.graphs_get(self._value_id, self._idx)
        if range is not None or len(shape) != 3:
            raise RuntimeError("Invalid graph data.")

        if shape[2] == 4:
            dtype = np.float32
        elif shape[2] == 8:
            dtype = np.float64
        else:
            raise RuntimeError("Invalid graph datatype.")

        return np.frombuffer(data, dtype=dtype).reshape(shape[:2])


class GraphRange(_GraphBase):
    """Graph UI element with range."""

    def add_points(self, points: Buffer, range: tuple[float, float], update: bool = False) -> None:
        """Add the points to the graph.

        Args:
            points(Buffer): The points to add. Has to implement the buffer protocol.
            range(tuple[float, float]): The range of the graph
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._check()
        self._server.graphs_add_points(self._value_id, self._idx, points, update, range)

    def set(self, graph: Buffer, range: tuple[float, float], update: bool = False) -> None:
        """Set the graph to the UI graphs.

        Args:
            graph(Buffer): The graph to set. Has to implement the buffer protocol.
            range(tuple[float, float]): The range of the graph.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._check()
        self._server.graphs_set(self._value_id, self._idx, graph, update, range)

    def get(self) -> tuple[np.ndarray, tuple[float, float]]:
        """Get the graph from the UI graphs.

        Returns:
            tuple[np.ndarray, tuple[float, float]]: The graph and the range.
        """
        data, shape, range = self._server.graphs_get(self._value_id, self._idx)
        if range is None or len(shape) != 2:
            raise RuntimeError("Invalid graph data.")

        if shape[1] == 4:
            dtype = np.float32
        elif shape[1] == 8:
            dtype = np.float64
        else:
            raise RuntimeError("Invalid graph datatype.")

        return np.frombuffer(data, dtype=dtype), range


class ValueGraphs(_StaticBase):
    """Graph UI element."""

    def __init__(self, counter: _Counter):  # noqa: D107
        super().__init__(counter)

        self._graphs: dict[int, _GraphBase] = {}
        self.__getitem__ = self.get

    def get(self, idx: int) -> _GraphBase:
        """Get the graph by index.

        Args:
            idx(int): The index of the graph.

        Returns:
            _Graph: The graph object.
        """
        return self._graphs[idx]

    def set(
        self, graph: Buffer, idx: int | None = None, range: tuple[float, float] | None = None, update: bool = False
    ) -> _GraphBase:
        """Set the graph to the UI graphs. Returns existing graph if the index is already used.

        Two options for the graph data:
        - Data with shape (2, N) where the first row is the x values and the second row is the y values.
        - Data with shape (N,) where the x values are generated automatically from the range parameter.

        Args:
            graph(Buffer): The graph to set. Has to implement the buffer protocol (numpy array).
            idx(int, optional): The index of the graph. If None, smallest available index is used. Defaults to None.
            range(tuple[float, float], optional): The range of the graph. Defaults to None.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        if idx is None:
            idx = 0
            while idx in self._graphs:
                idx += 1
        elif idx in self._graphs:
            self._graphs[idx]._kill()

        self._server.graphs_set(self._value_id, idx, graph, update, range)
        if range is None:
            graph_obj = Graph(self._value_id, idx, self._server)
        else:
            graph_obj = GraphRange(self._value_id, idx, self._server)
        self._graphs[idx] = graph_obj
        return graph_obj

    def remove(self, graph: _GraphBase, update: bool = False) -> None:
        """Remove the graph from the UI graphs.

        Args:
            graph(_Graph): The graph object.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        graph._kill()
        if graph.idx in self._graphs:
            graph._kill()
            self._server.graphs_remove(self._value_id, graph.idx, update)
            self._graphs.pop(graph.idx)

    def remove_idx(self, idx: int, update: bool = False) -> None:
        """Remove the graph from the UI graphs.

        Args:
            idx(int): The index of the graph.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        if idx in self._graphs:
            self._graphs[idx]._kill()
            self._server.graphs_remove(self._value_id, idx, update)
            self._graphs.pop(idx)

    def clear(self, update: bool = False) -> None:
        """Clear the all UI graphs.

        Args:
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._graphs.clear()
        self._server.graphs_clear(self._value_id, update)
