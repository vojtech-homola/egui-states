# ruff: noqa: D107
from abc import ABC, abstractmethod
from collections.abc import Buffer, Callable
from typing import Any

import numpy as np

from egui_pysync.signals import SignalsManager
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


class ErrorSignal:
    """Error signal for processing errors from the state server."""

    def __init__(self, siganls_manager: SignalsManager):
        """Initialize the ErrorSignal."""
        self._value_id = 0
        self._signals_manager = siganls_manager

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

    def __init__(self, counter: _Counter) -> None:
        self._value_id = counter.get_id()

    def _initialize(self, server: SteteServerCoreBase):
        self._server = server


class _ValueBase(_StaticBase):
    _signals_manager: SignalsManager

    def _initialize(self, server: SteteServerCoreBase, signals_manager: SignalsManager):
        self._server = server
        self._signals_manager = signals_manager
        # signals_manager.register_signal(self._value_id)


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

    def connect(self, callback: Callable[[T], Any]) -> None:
        """Connect a callback to the value.

        Args:
            callback(Callable[[T], Any]): The callback to connect.
        """
        self._signals_manager.add_callback(self._value_id, callback)

    def disconnect(self, callback: Callable[[T], Any]) -> None:
        """Disconnect a callback from the value.

        Args:
            callback(Callable[[T], Any]): The callback to disconnect.
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


class Signal[T](_ValueBase):
    """Signal from UI."""

    def set(self, value: T) -> None:
        """Set the signal value.

        Signal is emitted to all connected callbacks.

        Args:
            value(T): The value to set.
        """
        self._server.signal_set(self._value_id, value)

    def connect(self, callback: Callable[[T], Any]) -> None:
        """Connect a callback to the signal.

        Args:
            callback(Callable[[], Any]): The callback to connect.
        """
        self._signals_manager.add_callback(self._value_id, callback)

    def disconnect(self, callback: Callable[[T], Any]) -> None:
        """Disconnect a callback from the value.

        Args:
            callback(Callable[[], Any]): The callback to disconnect.
        """
        self._signals_manager.remove_callback(self._value_id, callback)

    def disconnect_all(self) -> None:
        """Disconnect all callbacks from the signal."""
        self._signals_manager.clear_callbacks(self._value_id)


class SignalEmpty(_ValueBase):
    """Empty Signal from UI."""

    def set(self) -> None:
        """Set the signal value.

        Signal is emitted to all connected callbacks.
        """
        self._server.signal_set(self._value_id, None)

    def connect(self, callback: Callable[[], Any]) -> None:
        """Connect a callback to the signal.

        Args:
            callback(Callable[[], Any]): The callback to connect.
        """
        self._signals_manager.add_callback(self._value_id, callback)

    def disconnect(self, callback: Callable[[], Any]) -> None:
        """Disconnect a callback from the value.

        Args:
            callback(Callable[[], Any]): The callback to disconnect.
        """
        self._signals_manager.remove_callback(self._value_id, callback)

    def disconnect_all(self) -> None:
        """Disconnect all callbacks from the signal."""
        self._signals_manager.clear_callbacks(self._value_id)


class ValueImage(_StaticBase):
    """Image UI element."""

    def set(
        self,
        image: Buffer,
        origin: list[int] | tuple[int, int] | None = None,
        update: bool = False,
    ) -> None:
        """Set the image in the UI image.

        Args:
            image(Buffer): The image to set.
            origin(list[int] | tuple[int, int], optional): If set only inner rectangle with given origin (top, left).
                                                           Defaults to None.
            update(bool, optional): Whether to update the UI. Defaults to True.
        """
        self._server.image_set(self._value_id, image, update, origin)

    def get(self) -> np.ndarray:
        """Get the image in the UI image.

        Returns:
            np.ndarray: The image in the UI image. Stape is (height, width, 4). 4 is for RGBA.
        """
        data, shape = self._server.image_get(self._value_id)
        shape = (shape[0], shape[1], 4)

        return np.frombuffer(data, dtype=np.uint8).reshape(shape)

    def size(self) -> tuple[int, int]:
        """Get the size of the image.

        Returns:
            tuple[int, int]: The size of the image (height, width).
        """
        return self._server.image_size(self._value_id)


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


class Graph:
    """Graph UI element."""

    def __init__(self, value_id: int, idx: int, server: SteteServerCoreBase):
        """Initialize the Graph."""
        self._value_id = value_id
        self._idx = idx
        self._server = server

        self._deleted = False

    @property
    def idx(self) -> int:
        """Get the index of the graph in ValueGraphs."""
        return self._idx

    @property
    def allive(self) -> bool:
        """Check if the graph is allive."""
        return not self._deleted

    @property
    def is_linear(self) -> bool:
        """Check if the graph is linear -> only Y axis."""
        return self._server.graphs_is_linear(self._value_id, self._idx)

    def len(self) -> int:
        """Get the length of the graph.

        Returns:
            int: The length of the graph.
        """
        self._check()
        return self._server.graphs_len(self._value_id, self._idx)

    def add_points(self, points: Buffer, update: bool = False) -> None:
        """Add the points to the graph.

        Args:
            points(Buffer): The points to add. Has to implement the buffer protocol.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._check()
        self._server.graphs_add_points(self._value_id, self._idx, points, update)

    def set(self, graph: Buffer, update: bool = False) -> None:
        """Set the graph to the UI graphs.

        Args:
            graph(Buffer): The graph to set. Has to implement the buffer protocol.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._check()
        self._server.graphs_set(self._value_id, self._idx, graph, update)

    def get(self) -> np.ndarray:
        """Get the graph from the UI graphs.

        Returns:
            np.ndarray: The graph.
        """
        data, shape = self._server.graphs_get(self._value_id, self._idx)

        if shape[-1] == 4:
            dtype = np.float32
        elif shape[-1] == 8:
            dtype = np.float64
        else:
            raise RuntimeError("Invalid graph datatype.")

        reshape = shape[:2] if len(shape) == 3 else shape[:1]
        return np.frombuffer(data, dtype=dtype).reshape(reshape)

    def _kill(self):
        self._deleted = True
        self._server.graphs_remove(self._value_id, self._idx, update=False)

    def _check(self):
        if self._deleted:
            raise RuntimeError("Graph was deleted. You have to create a new one.")

    def __len__(self) -> int:
        """Get the length of the graph."""
        return self.len()


class ValueGraphs(_StaticBase):
    """Graph UI element."""

    def __init__(self, counter: _Counter):  # noqa: D107
        super().__init__(counter)

        self._graphs: dict[int, Graph] = {}
        self.__getitem__ = self.get

    def get(self, idx: int) -> Graph:
        """Get the graph by index.

        Args:
            idx(int): The index of the graph.

        Returns:
            _Graph: The graph object.
        """
        return self._graphs[idx]

    def set(self, graph: Buffer, idx: int | None = None, update: bool = False) -> Graph:
        """Set the graph to the UI graphs.

        If idx is specified and the graph with the index already exists, it will be updated.

        Two options for the graph data:
        - Data with shape (2, N) where the first row is the x values and the second row is the y values.
        - Data with shape (N,) where the x axis is considered to be linear.

        Args:
            graph(Buffer): The graph to set. Has to implement the buffer protocol (numpy array).
            idx(int, optional): The index of the graph. If None, smallest available index is used. Defaults to None.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        if idx is None:
            idx = 0
            while idx in self._graphs:
                idx += 1
        elif idx in self._graphs:
            existing_graph = self._graphs[idx]
            existing_graph.set(graph, update)
            return existing_graph

        self._server.graphs_set(self._value_id, idx, graph, update)
        graph_obj = Graph(self._value_id, idx, self._server)
        self._graphs[idx] = graph_obj
        return graph_obj

    def remove(self, graph: Graph, update: bool = False) -> None:
        """Remove the graph from the UI graphs.

        Args:
            graph(_Graph): The graph object.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
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
