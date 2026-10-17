# ruff: noqa: D107 D105 D102 PLC2801
"""Typed state handles used by generated Python server bindings."""

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

#: Gray, gray-alpha, RGB, or RGBA color whose components are in ``0..255``.
type ImageColor = int | tuple[int, int] | tuple[int, int, int] | tuple[int, int, int, int]


class _CustomStruct:
    __getitem__ = object.__getattribute__


class ISubStates(ABC):
    """Base class for a generated nested state group."""

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
        """Preserve every signaled change for server callbacks.

        Changes for one state are delivered in order. Callbacks for different
        states may still run concurrently when multiple workers are configured.
        """
        self._server.signal_set_to_queue(self._value_id)

    def signal_set_to_single(self) -> None:
        """Coalesce pending signaled changes to the latest value.

        This is the default mode.
        """
        self._server.signal_set_to_single(self._value_id)


class Value[T](_SignalBase):
    """Bidirectional value stored by both the server and egui client."""

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
        """Replace the value and send it to the client.

        Args:
            value (T): Value to store locally and send to the client.
            set_signal (bool, optional): Whether to emit callbacks for the new
                value. Defaults to False.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.value_set(self._value_id, value, set_signal, update)

    def get(self) -> T:
        """Return the server's current value.

        Returns:
            T: The current value.
        """
        return self._server.value_get(self._value_id)

    def connect(self, callback: Callable[[T], Any]) -> None:
        """Connect a callback to the value.

        Args:
            callback (Callable[[T], Any]): The callback to connect.
        """
        self._signals_manager.add_callback(self._value_id, callback)

    def connect_previous(self, callback: Callable[[T, T], Any]) -> None:
        """Connect a callback which also receives the previous value.

        The previous value is the one the callback was last notified about, not
        necessarily the immediate predecessor: in single mode successive changes
        coalesce, so a -> b -> c delivering only c calls the callback with
        (c, a).

        Args:
            callback (Callable[[T, T], Any]): The callback to connect. It is
                called with the new value followed by the previous one.
        """
        self._signals_manager.add_callback_previous(self._value_id, callback)

    def disconnect(self, callback: Callable[[T], Any] | Callable[[T, T], Any]) -> None:
        """Disconnect a callback from the value.

        Takes callbacks connected either way, so there is nothing to match up
        against how the callback was connected.

        Args:
            callback (Callable[[T], Any] | Callable[[T, T], Any]): The callback
                to disconnect.
        """
        self._signals_manager.remove_callback(self._value_id, callback)
        self._signals_manager.remove_callback_previous(self._value_id, callback)

    def disconnect_all(self) -> None:
        """Disconnect all callbacks from the value, of both variants."""
        self._signals_manager.clear_callbacks(self._value_id)


class ValueTake[T](_StaticBase):
    """One-shot value sent to and consumed once by the client.

    Unlike :class:`Value`, this is not readable as persistent server state. It
    is the server-to-client counterpart of :class:`Signal`.
    """

    def __init__(self, obj_id: int) -> None:
        self._obj_id = obj_id

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_value_take(name, types[self._obj_id])
        del self._obj_id

    def set(self, value: T, blocking: bool = False, update: bool = False) -> None:
        """Send a one-shot value to the client.

        Args:
            value (T): The value to set.
            blocking (bool, optional): Whether the next send waits until the
                client consumes this value. Defaults to False.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.value_take_set(self._value_id, value, blocking, update)


class ValueTakeEmpty(_StaticBase):
    """Unit-valued one-shot event sent to and consumed once by the client."""

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_value_take(name, emp)

    def set(self, blocking: bool = False, update: bool = False) -> None:
        """Set the value of the UI element.

        Args:
            blocking (bool, optional): Whether the next send waits until the
                client consumes this value. Defaults to False.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.value_take_set(self._value_id, (), blocking, update)


class Static[T](_StaticBase):
    """Server-controlled value that the egui client can read but not modify."""

    def __init__(self, obj_id: int, initial_value: T) -> None:
        self._initial_value = initial_value
        self._obj_id = obj_id

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_static(name, types[self._obj_id], self._initial_value)
        del self._initial_value
        del self._obj_id

    def set(self, value: T, update: bool = False) -> None:
        """Replace the value mirrored to the client.

        Args:
            value (T): The value to set.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.static_set(self._value_id, value, update)

    def get(self) -> T:
        """Get the static value of the UI.

        Returns:
            T: The static value.
        """
        return self._server.static_get(self._value_id)


class Signal[T](_SignalBase):
    """Client-to-server event that does not retain a client-side value.

    Calling :meth:`set` on the server emits the event to local callbacks.
    """

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
            value (T): The value to set.
        """
        self._server.signal_set(self._value_id, value)

    def connect(self, callback: Callable[[T], Any]) -> None:
        """Connect a callback to the signal.

        Args:
            callback (Callable[[T], Any]): Callback receiving the signal value.
        """
        self._signals_manager.add_callback(self._value_id, callback)

    def disconnect(self, callback: Callable[[T], Any]) -> None:
        """Disconnect a callback from the value.

        Args:
            callback (Callable[[T], Any]): Previously connected callback.
        """
        self._signals_manager.remove_callback(self._value_id, callback)

    def disconnect_all(self) -> None:
        """Disconnect all callbacks from the signal."""
        self._signals_manager.clear_callbacks(self._value_id)


class SignalEmpty(_SignalBase):
    """Unit-valued client-to-server event with no payload argument."""

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
            callback (Callable[[], Any]): The callback to connect.
        """
        self._signals_manager.add_callback(self._value_id, callback)

    def disconnect(self, callback: Callable[[], Any]) -> None:
        """Disconnect a callback from the value.

        Args:
            callback (Callable[[], Any]): The callback to disconnect.
        """
        self._signals_manager.remove_callback(self._value_id, callback)

    def disconnect_all(self) -> None:
        """Disconnect all callbacks from the signal."""
        self._signals_manager.clear_callbacks(self._value_id)


class Image(_StaticBase):
    """Server-controlled image mirrored to an initialized client texture.

    Each dimension is at most 65,536 pixels, inclusive. Oversized images raise
    ``ValueError``. Client renderer and memory limits may be smaller.
    """

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_image(name)

    def set(
        self,
        image: Buffer,
        update: bool = False,
    ) -> None:
        """Set the image in the UI image.

        Args:
            image (Buffer): The image to set.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.image_set(self._value_id, image, update)

    def update(
        self,
        image: Buffer,
        origin: list[int] | tuple[int, int],
        update: bool = False,
        force: bool = False,
    ) -> None:
        """Update a rectangular part of the image.

        Args:
            image (Buffer): The image rectangle to write.
            origin (list[int] | tuple[int, int]): Top-left origin as ``(y, x)``.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
            force (bool, optional): Whether to replace a pending update for the
                same rectangle. Defaults to False.
        """
        self._server.image_update(self._value_id, image, origin, update, force)

    def set_all(
        self,
        shape: list[int] | tuple[int, int],
        color: ImageColor,
        update: bool = False,
    ) -> None:
        """Fill the complete image with one color using a compact message.

        Args:
            shape (list[int] | tuple[int, int]): Image shape as ``(height,
                width)``.
            color (ImageColor): Gray, gray-alpha, RGB, or RGBA color. Every
                component must be in ``0..255``.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.image_set_all(self._value_id, shape, color, update)

    def get(self) -> npt.NDArray[np.uint8]:
        """Get the image in the UI image.

        Returns:
            npt.NDArray[np.uint8]: RGBA image with shape ``(height, width, 4)``.
        """
        data, shape = self._server.image_get(self._value_id)
        shape = (shape[0], shape[1], 4)

        return np.frombuffer(data, dtype=np.uint8).reshape(shape)

    def shape(self) -> tuple[int, int]:
        """Get the shape of the image.

        Returns:
            tuple[int, int]: Image shape as ``(height, width)``.
        """
        return self._server.image_size(self._value_id)


class SingleImage:
    """Handle for one indexed image within an :class:`ImageMulti` state.

    Each dimension is at most 65,536 pixels, inclusive. Oversized images raise
    ``ValueError``. Client renderer and memory limits may be smaller.
    """

    def __init__(self, server: StateServerCore, value_id: int, index: int) -> None:
        self._server = server
        self._value_id = value_id
        self._index = index

    def set(self, image: Buffer, update: bool = False) -> None:
        """Create or replace this indexed image.

        Args:
            image (Buffer): Image data shaped as ``(height, width)`` or
                ``(height, width, channels)``.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.image_multi_set(self._value_id, self._index, image, update)

    def update(
        self,
        image: Buffer,
        origin: list[int] | tuple[int, int],
        update: bool = False,
        force: bool = False,
    ) -> None:
        """Update a rectangular part of this existing indexed image.

        Args:
            image (Buffer): The image rectangle to write.
            origin (list[int] | tuple[int, int]): Top-left origin as ``(y, x)``.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
            force (bool, optional): Whether to replace a pending update for the
                same image and rectangle. Defaults to False.
        """
        self._server.image_multi_update(self._value_id, self._index, image, origin, update, force)

    def set_all(
        self,
        shape: list[int] | tuple[int, int],
        color: ImageColor,
        update: bool = False,
    ) -> None:
        """Create or replace this indexed image with one color.

        Args:
            shape (list[int] | tuple[int, int]): Image shape as ``(height,
                width)``.
            color (ImageColor): Gray, gray-alpha, RGB, or RGBA color.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.image_multi_set_all(self._value_id, self._index, shape, color, update)

    def get(self) -> npt.NDArray[np.uint8]:
        """Return this indexed image.

        Returns:
            npt.NDArray[np.uint8]: RGBA image with shape ``(height, width, 4)``.
        """
        data, shape = self._server.image_multi_get(self._value_id, self._index)
        return np.frombuffer(data, dtype=np.uint8).reshape((shape[0], shape[1], 4))

    def shape(self) -> tuple[int, int]:
        """Return this indexed image shape.

        Returns:
            tuple[int, int]: Image shape as ``(height, width)``.
        """
        return self._server.image_multi_size(self._value_id, self._index)


class ImageMulti(_StaticBase):
    """Sparse collection of server-controlled images indexed by integers.

    Each image dimension is limited to 65,536 pixels, inclusive. Client renderer
    and memory limits may be smaller.
    """

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_image_multi(name)

    def get(self, index: int) -> SingleImage:
        """Return a handle for one image index.

        Args:
            index (int): Non-negative image index.

        Returns:
            SingleImage: Handle that can create, update, or read ``index``.
        """
        if not isinstance(index, int):
            raise TypeError("index must be an integer")
        return SingleImage(self._server, self._value_id, index)

    def remove_index(self, index: int, update: bool = False) -> None:
        """Remove an image index; an absent index is a no-op.

        Args:
            index (int): Non-negative image index.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.image_multi_remove_index(self._value_id, index, update)

    def reset(self, update: bool = False) -> None:
        """Remove every populated image index.

        Args:
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.image_multi_reset(self._value_id, update)

    def indices(self) -> list[int]:
        """Return populated indices.

        Returns:
            list[int]: Populated indices in ascending order.
        """
        return self._server.image_multi_indices(self._value_id)

    def __len__(self) -> int:
        return self._server.image_multi_len(self._value_id)

    def __contains__(self, index: object) -> bool:
        if not isinstance(index, int):
            return False
        return self._server.image_multi_contains(self._value_id, index)

    def __getitem__(self, index: int) -> SingleImage:
        return self.get(index)


class Map[K, V](_StaticBase):
    """Server-controlled dictionary mirrored read-only to the client."""

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
            value (dict[K, V]): The dictionary to set.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.map_set(self._value_id, value, update)

    def get(self) -> dict[K, V]:
        """Get the dict in the UI dict.

        Returns:
            dict[K, V]: The dictionary in the UI state.
        """
        return self._server.map_get(self._value_id)

    def set_item(self, key: K, value: V, update: bool = False) -> None:
        """Set the item in the UI dict.

        Args:
            key (K): The key of the item.
            value (V): The value of the item.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.map_set_item(self._value_id, key, value, update)

    def get_item(self, key: K) -> V:
        """Get the item in the UI dict.

        Args:
            key (K): The key of the item.

        Returns:
            V: The value of the item.
        """
        return self._server.map_get_item(self._value_id, key)

    def remove_item(self, key: K, update: bool = False) -> None:
        """Remove the item from the UI dict.

        Args:
            key (K): The key of the item.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
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


class Vec[T](_StaticBase):
    """Server-controlled list mirrored read-only to the client."""

    def __init__(self, obj_id: int) -> None:
        self._obj_id = obj_id

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_vec(name, types[self._obj_id])
        del self._obj_id

    def set(self, value: list[T], update: bool = False) -> None:
        """Set the list in the UI list.

        Args:
            value (list[T]): The list to set.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.list_set(self._value_id, value, update)

    def get(self) -> list[T]:
        """Get the list in the UI list.

        Returns:
            list[T]: The list in the UI state.
        """
        return self._server.list_get(self._value_id)

    def set_item(self, idx: int, value: T, update: bool = False) -> None:
        """Set the item in the UI list.

        Args:
            idx (int): The index of the item.
            value (T): The value of the item.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.list_set_item(self._value_id, idx, value, update)

    def get_item(self, idx: int) -> T:
        """Get the item in the UI list.

        Args:
            idx (int): The index of the item.

        Returns:
            T: The value of the item.
        """
        return self._server.list_get_item(self._value_id, idx)

    def remove_item(self, idx: int, update: bool = False) -> None:
        """Remove the item from the UI list.

        Args:
            idx (int): The index of the item.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.list_del_item(self._value_id, idx, update)

    def add_item(self, value: T, update: bool = False) -> None:
        """Add the item to the UI list.

        Args:
            value (T): The value of the item.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
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


def _get_dtype_id(dtype: type[np.generic]) -> int:
    t = _DTYPE_TO_ID.get(np.dtype(dtype).type)
    if t is None:
        raise ValueError(f"Unsupported dtype: {dtype}")
    return t


class Data[T: np.generic](_StaticBase):
    """Contiguous NumPy-compatible numeric buffer mirrored to the client.

    Args:
        dtype (type[T]): One of ``numpy.uint8`` through ``numpy.uint64``,
            ``numpy.int8`` through ``numpy.int64``, ``numpy.float32``, or
            ``numpy.float64``.
    """

    def __init__(self, dtype: type[T]) -> None:
        self._dtype = dtype

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_data(name, _get_dtype_id(self._dtype))

    def get(self) -> npt.NDArray[T]:
        """Return a locally mutable snapshot of the current server buffer.

        Mutating the returned array does not update the server state or client;
        call :meth:`set` to publish modified data.

        Returns:
            npt.NDArray[T]: A NumPy view backed by a copied ``bytearray``.
        """
        data = self._server.data_get(self._value_id)
        return np.frombuffer(data, dtype=self._dtype)

    def set(self, data: Buffer, update: bool = False) -> None:
        """Set the data in the UI data.

        Args:
            data (Buffer): Data to set. NumPy arrays and other buffer-protocol
                objects are accepted.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_set(self._value_id, data, update)

    def add(self, data: Buffer, update: bool = False) -> None:
        """Add the data to the UI data.

        Args:
            data (Buffer): Data to append. NumPy arrays and other
                buffer-protocol objects are accepted.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_add(self._value_id, data, update)

    def replace(self, data: Buffer, index: int, update: bool = False) -> None:
        """Replace the data in the UI data.

        Args:
            data (Buffer): Replacement data. NumPy arrays and other
                buffer-protocol objects are accepted.
            index (int): Index at which replacement starts.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_replace(self._value_id, data, index, update)

    def remove(self, index: int, count: int, update: bool = False) -> None:
        """Remove the data from the UI data.

        Args:
            index (int): Index at which removal starts.
            count (int): Number of items to remove.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_remove(self._value_id, index, count, update)

    def clear(self, update: bool = False) -> None:
        """Clear the data in the UI data.

        Args:
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_clear(self._value_id, update)


class DataTake[T: np.generic](_StaticBase):
    """One-shot numeric buffer sent to and consumed by the client.

    Args:
        dtype (type[T]): One of the integer or floating-point NumPy scalar
            types supported by :class:`Data`.
    """

    def __init__(self, dtype: type[T]) -> None:
        self._dtype = dtype

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_data_take(name, _get_dtype_id(self._dtype))

    def set(
        self,
        data: Buffer,
        blocking: bool = False,
        update: bool = False,
        cache: bool = False,
    ) -> None:
        """Set the data in the UI DataTake.

        DataTake does not have a get method because the data is not stored in
        the server. It is an alternative to Signal with the opposite transport
        direction.

        Args:
            data (Buffer): Data to send. NumPy arrays and other buffer-protocol
                objects are accepted.
            blocking (bool, optional): Whether the next send waits until the
                client consumes this data. Defaults to False.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
            cache (bool, optional): Whether to cache the last sent data so a
                newly initialized client receives it. Defaults to False.
        """
        self._server.data_take_set(self._value_id, data, blocking, update, cache)


class SingleData[T: np.generic]:
    """Handle for one indexed buffer within a :class:`DataMulti` state.

    Instances are returned by :meth:`DataMulti.get` or indexed access; users
    do not normally construct them directly.
    """

    def __init__(self, dtype: type[T], server: StateServerCore, value_id: int, index: int) -> None:
        self._dtype = dtype
        self._server = server
        self._value_id = value_id
        self._index = index

    def get(self) -> npt.NDArray[T]:
        """Return a locally mutable snapshot of the buffer at this index.

        Mutating the array does not update the server state or client; call
        :meth:`set` to publish modified data.

        Returns:
            npt.NDArray[T]: A NumPy view backed by a copied ``bytearray``.
        """
        data = self._server.data_multi_get(self._value_id, self._index)
        return np.frombuffer(data, dtype=self._dtype)

    def set(self, data: Buffer, update: bool = False) -> None:
        """Set the data in the UI data at this index.

        Args:
            data (Buffer): Data to set. NumPy arrays and other buffer-protocol
                objects are accepted.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_multi_set(self._value_id, self._index, data, update)

    def add(self, data: Buffer, update: bool = False) -> None:
        """Add the data to the UI data at this index.

        Args:
            data (Buffer): Data to append. NumPy arrays and other
                buffer-protocol objects are accepted.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_multi_add(self._value_id, self._index, data, update)

    def replace(self, data: Buffer, index: int, update: bool = False) -> None:
        """Replace the data in the UI data at this index.

        Args:
            data (Buffer): Replacement data. NumPy arrays and other
                buffer-protocol objects are accepted.
            index (int): Index at which replacement starts within this buffer.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_multi_replace(self._value_id, self._index, data, index, update)

    def remove(self, index: int, count: int, update: bool = False) -> None:
        """Remove the data from the UI data at this index.

        Args:
            index (int): Index at which removal starts within this buffer.
            count (int): Number of items to remove.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_multi_remove(self._value_id, self._index, index, count, update)

    def clear(self, update: bool = False) -> None:
        """Clear the data in the UI data at this index.

        Args:
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_multi_clear(self._value_id, self._index, update)


class DataMulti[T: np.generic](_StaticBase):
    """Collection of numeric buffers indexed by non-negative integers.

    Args:
        dtype (type[T]): One of the integer or floating-point NumPy scalar
            types supported by :class:`Data`.
    """

    def __init__(self, dtype: type[T]) -> None:
        self._dtype = dtype

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_data_multi(name, _get_dtype_id(self._dtype))

    def get(self, index: int) -> SingleData[T]:
        """Get the SingleData object for the given index.

        Args:
            index (int): The index of the ``SingleData`` object.

        Returns:
            SingleData[T]: The ``SingleData`` object for the given index.
        """
        return SingleData(self._dtype, self._server, self._value_id, index)

    def remove_index(self, index: int, update: bool = False) -> None:
        """Remove the given index from the DataMulti.

        Args:
            index (int): The index to remove.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_multi_remove_index(self._value_id, index, update)

    def reset(self, update: bool = False) -> None:
        """Reset (clear all indices) in the DataMulti.

        Args:
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_multi_reset(self._value_id, update)

    def __getitem__(self, index: int) -> SingleData[T]:
        if isinstance(index, int):
            return self.get(index)
        raise TypeError("index must be an integer")


class SingleDataTake[T: np.generic]:
    """Handle for one indexed transfer within a :class:`DataMultiTake` state.

    Instances are returned by :meth:`DataMultiTake.get` or indexed access;
    users do not normally construct them directly.
    """

    def __init__(self, dtype: type[T], server: StateServerCore, value_id: int, index: int) -> None:
        self._dtype = dtype
        self._server = server
        self._value_id = value_id
        self._index = index

    def set(self, data: Buffer, blocking: bool = False, update: bool = False, cache: bool = False) -> None:
        """Set the data in the UI DataMultiTake at this index.

        Args:
            data (Buffer): Data to send. NumPy arrays and other buffer-protocol
                objects are accepted.
            blocking (bool, optional): Whether the next send waits until the
                client consumes this data. Defaults to False.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
            cache (bool, optional): Whether to cache the last sent data for a
                newly initialized client. Defaults to False.
        """
        self._server.data_multi_take_set(
            self._value_id,
            self._index,
            data,
            blocking,
            update,
            cache,
        )


class DataMultiTake[T: np.generic](_StaticBase):
    """Keyed one-shot numeric buffers sent to and consumed by the client.

    Args:
        dtype (type[T]): One of the integer or floating-point NumPy scalar
            types supported by :class:`Data`.
    """

    def __init__(self, dtype: type[T]) -> None:
        self._dtype = dtype

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_data_multi_take(
            name,
            _get_dtype_id(self._dtype),
        )

    def get(self, index: int) -> SingleDataTake[T]:
        """Get the SingleDataTake object for the given index.

        Args:
            index (int): The index of the ``SingleDataTake`` object.

        Returns:
            SingleDataTake[T]: The ``SingleDataTake`` object for the given
                index.
        """
        return SingleDataTake(self._dtype, self._server, self._value_id, index)

    def remove_index(self, index: int, update: bool = False) -> None:
        """Remove the given index from the DataMultiTake.

        Args:
            index (int): The index to remove.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_multi_take_remove_index(self._value_id, index, update)

    def reset(self, update: bool = False) -> None:
        """Reset (clear all indices) in the DataMultiTake.

        Args:
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_multi_take_reset(self._value_id, update)

    def __getitem__(self, index: int) -> SingleDataTake[T]:
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
    "ImageColor",
    "_CustomStruct",
    "PyObjectType",
]
