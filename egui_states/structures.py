# ruff: noqa: D107 D105 D102 PLC2801
from __future__ import annotations

from abc import ABC, abstractmethod
from collections.abc import Buffer, Callable
from typing import Any

import numpy as np
import numpy.typing as npt

from egui_states import _core
from egui_states._core import (
    PyObjectType,
    StateServerCore,
    bo,
    cl,
    emp,
    enu,
    f32,
    f64,
    i8,
    i16,
    i32,
    i64,
    li,
    map,
    opt,
    st,
    tu,
    u8,
    u16,
    u32,
    u64,
    vec,
)
from egui_states.signals import SignalsManager


class _CustomStruct:
    __getitem__ = object.__getattribute__


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
        self._server.signal_set_to_queue(self._value_id)

    def signal_set_to_single(self) -> None:
        """Set the value to single mode. It is the default mode.

        In single mode, only the last change of the value is processed.
        """
        self._server.signal_set_to_single(self._value_id)


class Value[T](_SignalBase):
    """General UI value of type T."""

    def __init__(self, obj_id: int, initial_value: T, queue: bool = False) -> None:
        self._initial_value = initial_value
        self._obj_id = obj_id
        self._queue = queue

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_value(name, types[self._obj_id], self._initial_value, self._queue)
        del self._initial_value
        del self._obj_id
        del self._queue

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


class ValueTake[T](_StaticBase):
    """ValueTake is a value which can be taken in the UI only once.

    ValueTake does not have a get method, because the value is not stored in the server. It is alternative to Signal,
    but with opposite transport direction.
    """

    def __init__(self, obj_id: int) -> None:
        self._obj_id = obj_id

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_value_take(name, types[self._obj_id])
        del self._obj_id

    def set(self, value: T, blocking: bool = False, update: bool = False) -> None:
        """Set the value of the UI element.

        Args:
            value(T): The value to set.
            blocking(bool, optional): Whether the sending a new value with next call waits for acknowledgment from UI.
                Defaults to False.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.value_take_set(self._value_id, value, blocking, update)


class ValueTakeEmpty(_StaticBase):
    """ValueTakeEmpty is a value which can be taken in the UI only once.

    It is alternative to SignalEmpty, but with opposite transport direction.
    """

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_value_take(name, emp)

    def set(self, blocking: bool = False, update: bool = False) -> None:
        """Set the value of the UI element.

        Args:
            blocking(bool, optional): Whether the sending a new value with next call waits for acknowledgment from UI.
                Defaults to False.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.value_take_set(self._value_id, (), blocking, update)


class Static[T](_StaticBase):
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

    def __init__(self, obj_id: int, queue: bool = False) -> None:
        self._obj_id = obj_id
        self._queue = queue

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_signal(name, types[self._obj_id], self._queue)
        del self._obj_id
        del self._queue

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

    def __init__(self, queue: bool = False) -> None:
        self._queue = queue

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_signal(name, _core.emp, self._queue)
        del self._queue

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
            update(bool, optional): Whether to update the UI. Defaults to False.
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

    def __init__(self, key_id: int, value_id: int) -> None:
        self._key_id = key_id
        self._value_type_id = value_id

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_map(name, types[self._key_id], types[self._value_type_id])
        del self._key_id
        del self._value_type_id

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


class ValueVec[T](_StaticBase):
    """Vec UI element."""

    def __init__(self, obj_id: int) -> None:
        self._obj_id = obj_id

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_vec(name, types[self._obj_id])
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


_DTYPE_TO_ID = {
    np.uint8: 0,
    np.uint16: 1,
    np.uint32: 2,
    np.uint64: 3,
    np.int8: 4,
    np.int16: 5,
    np.int32: 6,
    np.int64: 7,
    np.float32: 8,
    np.float64: 9,
}


class Data[T: np.generic](_StaticBase):
    def __init__(self, dtype: type[T]) -> None:
        self._dtype = dtype

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_data(name, _DTYPE_TO_ID[np.dtype(self._dtype).type])

    def get(self) -> npt.NDArray[T]:
        """Get the data from the UI data.

        Returns:
            npt.NDArray[T]: The data in the UI data.
        """
        data = self._server.data_get(self._value_id)
        return np.frombuffer(data, dtype=self._dtype)

    def set(self, data: Buffer, update: bool = False) -> None:
        """Set the data in the UI data.

        Args:
            data(Buffer): The data to set. Has to implement the buffer protocol (numpy array).
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.data_set(self._value_id, data, update)

    def add(self, data: Buffer, update: bool = False) -> None:
        """Add the data to the UI data.

        Args:
            data(Buffer): The data to add. Has to implement the buffer protocol (numpy array).
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.data_add(self._value_id, data, update)

    def replace(self, data: Buffer, index: int, update: bool = False) -> None:
        """Replace the data in the UI data.

        Args:
            data(Buffer): The data to replace. Has to implement the buffer protocol (numpy array).
            index(int): The index of the data to replace.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.data_replace(self._value_id, data, index, update)

    def remove(self, index: int, count: int, update: bool = False) -> None:
        """Remove the data from the UI data.

        Args:
            index(int): The index of the data to remove.
            count(int): The number of data to remove.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.data_remove(self._value_id, index, count, update)

    def clear(self, update: bool = False) -> None:
        """Clear the data in the UI data.

        Args:
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.data_clear(self._value_id, update)


class SingleData[T: np.generic]:
    def __init__(self, dtype: type[T], server: StateServerCore, value_id: int, index: int) -> None:
        self._dtype = dtype
        self._server = server
        self._value_id = value_id
        self._index = index

    def get(self) -> npt.NDArray[T]:
        """Get the data from the UI data at this index.

        Returns:
            npt.NDArray[T]: The data in the UI data at this index.
        """
        data = self._server.data_multi_get(self._value_id, self._index)
        return np.frombuffer(data, dtype=self._dtype)

    def set(self, data: Buffer, update: bool = False) -> None:
        """Set the data in the UI data at this index.

        Args:
            data(Buffer): The data to set. Has to implement the buffer protocol (numpy array).
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.data_multi_set(self._value_id, self._index, data, update)

    def add(self, data: Buffer, update: bool = False) -> None:
        """Add the data to the UI data at this index.

        Args:
            data(Buffer): The data to add. Has to implement the buffer protocol (numpy array).
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.data_multi_add(self._value_id, self._index, data, update)

    def replace(self, data: Buffer, index: int, update: bool = False) -> None:
        """Replace the data in the UI data at this index.

        Args:
            data(Buffer): The data to replace. Has to implement the buffer protocol (numpy array).
            index(int): The index of the data to replace within this single data.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.data_multi_replace(self._value_id, self._index, data, index, update)

    def remove(self, index: int, count: int, update: bool = False) -> None:
        """Remove the data from the UI data at this index.

        Args:
            index(int): The index of the data to remove within this single data.
            count(int): The number of data items to remove.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.data_multi_remove(self._value_id, self._index, index, count, update)

    def clear(self, update: bool = False) -> None:
        """Clear the data in the UI data at this index.

        Args:
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.data_multi_clear(self._value_id, self._index, update)


class DataMulti[T: np.generic](_StaticBase):
    def __init__(self, dtype: type[T]) -> None:
        self._dtype = dtype

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_data_multi(name, _DTYPE_TO_ID[np.dtype(self._dtype).type])

    def get(self, index: int) -> SingleData[T]:
        """Get the SingleData object for the given index.

        Args:
            index(int): The index of the SingleData object.

        Returns:
            SingleData[T]: The SingleData object for the given index.
        """
        return SingleData(self._dtype, self._server, self._value_id, index)

    def remove_index(self, index: int, update: bool = False) -> None:
        """Remove the given index from the DataMulti.

        Args:
            index(int): The index to remove.
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.data_multi_remove_index(self._value_id, index, update)

    def reset(self, update: bool = False) -> None:
        """Reset (clear all indices) in the DataMulti.

        Args:
            update(bool, optional): Whether to update the UI. Defaults to False.
        """
        self._server.data_multi_reset(self._value_id, update)

    def __getitem__(self, index: int) -> SingleData[T]:
        if isinstance(index, int):
            return self.get(index)
        raise TypeError("index must be an integer")


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
    "bo",
    "emp",
    "enu",
    "cl",
    "st",
    "vec",
    "opt",
    "li",
    "tu",
    "map",
    "_CustomStruct",
]
