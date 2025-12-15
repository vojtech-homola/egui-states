# ruff: noqa: D107 D101 D105 D102 PLC2801
from abc import ABC, abstractmethod
from collections.abc import Buffer, Callable
from enum import Enum
from typing import Any, Self

import numpy as np
import numpy.typing as npt

from egui_states import _core
from egui_states._core import (
    PyObjectType,
    StateServerCore,
    i8,
    i16,
    i32,
    i64,
    u8,
    u16,
    u32,
    u64,
    f32,
    f64,
    emp,
    enu,
    cl,
    st,
    vec,
    opt,
    li,
    map,
)
from egui_states.signals import SignalsManager


class FastEnum(Enum):
    def __init_subclass__(cls):
        super().__init_subclass__()
        cls._member_list = tuple(cls)

    @classmethod
    def from_index(cls, index) -> Self:
        return cls._member_list[index]

    @classmethod
    def _get_members(cls) -> list[tuple[str, int]]:
        return [(member.name, member.value) for member in cls._member_list]

    def index(self) -> int:
        return self._member_list.index(self)


class CustomStruct:
    __getitem__ = object.__getattribute__

    def _get_values(self) -> list[Any]:
        return [self.__getattribute__(name) for name in self.__annotations__.keys()]

    @classmethod
    def _field_names(cls) -> list[str]:
        return list(cls.__annotations__.keys())


class ISubStates(ABC):
    """The base class for substates in the UI states."""

    @abstractmethod
    def __init__(self, parent: str) -> None:
        pass


class _StaticBase(ABC):
    _server: StateServerCore
    _value_id: int

    def _initialize_base(self, server: StateServerCore) -> None:
        self._server = server

    @abstractmethod
    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        pass


class _SignalBase(_StaticBase):
    _signals_manager: SignalsManager

    def _initialize_signal(self, signals_manager: SignalsManager) -> None:
        self._signals_manager = signals_manager

    def signal_set_to_queue(self) -> None:
        """Set the value to queue mode.

        In queue mode, changes of the value are queued and are all processed with single thread.
        """
        self._server.signal_set_to_multi(self._value_id)

    def signal_set_to_single(self) -> None:
        """Set the value to single mode. It is the default mode.

        In single mode, only the last change of the value is processed.
        """
        self._server.signal_set_to_single(self._value_id)


class Value[T](_SignalBase):
    """General UI value of type T."""

    def __init__(self, obj_id: int, initial_value: T):
        self._initial_value = initial_value
        self._obj_id = obj_id

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_value(name, types[self._obj_id], self._initial_value)
        del self._initial_value
        del self._obj_id

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

    def __init__(self, obj_id: int, initial_value: T) -> None:
        self._initial_value = initial_value
        self._obj_id = obj_id

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_static(name, types[self._obj_id], self._initial_value)
        del self._initial_value
        del self._obj_id

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


class Signal[T](_SignalBase):
    """Signal from UI."""

    def __init__(self, obj_id: int) -> None:
        self._obj_id = obj_id

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_signal(name, types[self._obj_id])
        del self._obj_id

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


class SignalEmpty(_SignalBase):
    """Empty Signal from UI."""

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_signal(name, _core.emp)

    def set(self) -> None:
        """Set the signal value.

        Signal is emitted to all connected callbacks.
        """
        self._server.signal_set(self._value_id, ())

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

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_image(name)

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

    def get(self) -> npt.NDArray[np.uint8]:
        """Get the image in the UI image.

        Returns:
            npt.NDArray[np.uint8]: The image in the UI image. Stape is (height, width, 4). 4 is for RGBA.
        """
        data, shape = self._server.image_get(self._value_id)
        shape = (shape[0], shape[1], 4)

        return np.frombuffer(data, dtype=np.uint8).reshape(shape)

    def shape(self) -> tuple[int, int]:
        """Get the shape of the image.

        Returns:
            tuple[int, int]: The shape of the image (height, width) or (y, x).
        """
        return self._server.image_size(self._value_id)


class ValueMap[K, V](_StaticBase):
    """Dict UI element."""

    def __init__(self, obj_id: int) -> None:
        self._obj_id = obj_id

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_map(name, types[self._obj_id])
        del self._obj_id

    def set(self, value: dict[K, V], update: bool = False) -> None:
        """Set the dict in the UI dict.

        Args:
            value(dict[K, V]): The dict to set.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.map_set(self._value_id, value, update)

    def get(self) -> dict[K, V]:
        """Get the dict in the UI dict.

        Returns:
            dict[K, V]: The dict in the UI dict.
        """
        return self._server.map_get(self._value_id)

    def set_item(self, key: K, value: V, update: bool = False) -> None:
        """Set the item in the UI dict.

        Args:
            key(K): The key of the item.
            value(V): The value of the item.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.map_set_item(self._value_id, key, value, update)

    def get_item(self, key: K) -> V:
        """Get the item in the UI dict.

        Args:
            key(K): The key of the item.

        Returns:
            V: The value of the item.
        """
        return self._server.map_get_item(self._value_id, key)

    def remove_item(self, key: K, update: bool = False) -> None:
        """Remove the item from the UI dict.

        Args:
            key(K): The key of the item.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.map_del_item(self._value_id, key, update)

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

    def __init__(self, obj_id: int) -> None:
        self._obj_id = obj_id

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_list(name, types[self._obj_id])
        del self._obj_id

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
        self._server.list_set_item(self._value_id, idx, value, update)

    def get_item(self, idx: int) -> T:
        """Get the item in the UI list.

        Args:
            idx(int): The index of the item.

        Returns:
            T: The value of the item.
        """
        return self._server.list_get_item(self._value_id, idx)

    def remove_item(self, idx: int, update: bool = False) -> None:
        """Remove the item from the UI list.

        Args:
            idx(int): The index of the item.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.list_del_item(self._value_id, idx, update)

    def add_item(self, value: T, update: bool = False) -> None:
        """Add the item to the UI list.

        Args:
            value(T): The value of the item.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.list_append_item(self._value_id, value, update)

    def __getitem__(self, idx: int) -> T:
        """Get the item in the UI list."""
        return self.get_item(idx)

    def __setitem__(self, idx: int, value: T) -> None:
        """Set the item in the UI list."""
        self.set_item(idx, value, update=False)


class Graph:
    """Graph UI element."""

    def __init__(self, value_id: int, idx: int, server: StateServerCore):
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

    def get(self) -> npt.NDArray[np.float32 | np.float64]:
        """Get the graph from the UI graphs.

        Returns:
            npt.NDArray[np.float32 | np.float64]: The graph.
        """
        data, byte_size, shape = self._server.graphs_get(self._value_id, self._idx)

        if byte_size == 4:
            dtype = np.float32
        elif byte_size == 8:
            dtype = np.float64
        else:
            raise RuntimeError("Invalid graph datatype.")

        reshape = shape if shape[0] == 2 else (shape[1],)
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


class ValueGraphs[T](_StaticBase):
    """Graph UI element."""

    def __init__(self, dtype: type[T]):  # noqa: D107
        if dtype == np.float32:
            self._id_double = False
        elif dtype == np.float64:
            self._id_double = True
        else:
            raise ValueError("Invalid dtype for graphs. Only np.float32 and np.float64 are supported.")

        self._graphs: dict[int, Graph] = {}
        self.__getitem__ = self.get

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_graphs(name, self._id_double)

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
        self._server.graphs_reset(self._value_id, update)


__all__ = [
    "i8",
    "i16",
    "i32",
    "i64",
    "u8",
    "u16",
    "u32",
    "u64",
    "f32",
    "f64",
    "emp",
    "enu",
    "cl",
    "st",
    "vec",
    "opt",
    "li",
    "map",
    "FastEnum",
    "CustomStruct",
]
