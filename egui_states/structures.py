# ruff: noqa: D107 D105 D102 PLC2801
"""Typed state handles used by generated Python server bindings."""

from __future__ import annotations

import os
from abc import ABC, abstractmethod
from collections.abc import Buffer, Callable, Sequence
from typing import Any, Final

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
from egui_states.transfer import TransferHandle

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


class _CallbackBase(_StaticBase):
    _signals_manager: SignalsManager

    def _initialize_signal(self, signals_manager: SignalsManager) -> None:
        self._signals_manager = signals_manager


class _SignalBase(_CallbackBase):
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
    """Bidirectional value stored by both the server and egui client.

    The server wins concurrent writes. Each `set` stays pending until the
    client acknowledges it, and a client write that arrives in the meantime is
    discarded without running callbacks; the client then takes the server's
    value. A value set continuously therefore cannot be changed reliably from
    the client.
    """

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

    Methods that take a buffer read it until they return, while other Python
    threads keep running. Do not modify the buffer from another thread during
    the call, or the client may receive a mix of old and new contents; pass a
    copy if another thread may write to it.
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

    Methods that take a buffer read it until they return, while other Python
    threads keep running. Do not modify the buffer from another thread during
    the call, or the client may receive a mix of old and new contents; pass a
    copy if another thread may write to it.
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

    def remove_item(self, key: K, update: bool = False) -> V:
        """Remove the item from the UI dict and return it.

        Args:
            key (K): The key of the item.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.

        Returns:
            V: The removed value.
        """
        return self._server.map_del_item(self._value_id, key, update)

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

    def remove_item(self, idx: int, update: bool = False) -> T:
        """Remove the item from the UI list and return it.

        Args:
            idx (int): The index of the item.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.

        Returns:
            T: The removed item.
        """
        return self._server.list_del_item(self._value_id, idx, update)

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


class _Unset:
    """Type of :data:`UNSET`, the absent-header default."""

    def __repr__(self) -> str:
        return "UNSET"


#: Default of the ``header`` arguments: no header is given, so the current one
#: is kept. ``None`` is not used for that, so an optional header type can still
#: send ``None``.
UNSET: Final = _Unset()


def _header(header: object) -> tuple[object] | None:
    return None if header is UNSET else (header,)


class Data[T: np.generic](_StaticBase):
    """Contiguous NumPy-compatible numeric buffer mirrored to the client.

    The data starts "not set": :meth:`get` returns ``None``, and the client
    shows no data, until :meth:`set` creates it, so an empty buffer differs
    from no data. Every other change needs data that is set and raises
    ``ValueError`` otherwise; :meth:`reset` returns the data to "not set".

    Methods that take a buffer read it until they return, while other Python
    threads keep running. Do not modify the buffer from another thread during
    the call, or the client may receive a mix of old and new contents; pass a
    copy if another thread may write to it.

    Args:
        dtype (type[T]): One of ``numpy.uint8`` through ``numpy.uint64``,
            ``numpy.int8`` through ``numpy.int64``, ``numpy.float32``, or
            ``numpy.float64``.
    """

    _header_id: int | None = None

    def __init__(self, dtype: type[T]) -> None:
        self._dtype = dtype

    def _header_type(self, types: list[PyObjectType]) -> PyObjectType | None:
        return None if self._header_id is None else types[self._header_id]

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_data(name, _get_dtype_id(self._dtype), self._header_type(types))

    def get(self) -> npt.NDArray[T] | None:
        """Return a locally mutable snapshot of the current server buffer.

        Mutating the returned array does not update the server state or client;
        call :meth:`set` to publish modified data.

        Returns:
            npt.NDArray[T] | None: A NumPy view backed by a copied
                ``bytearray``, or ``None`` while the data is not set.
        """
        data = self._server.data_get(self._value_id)
        return None if data is None else np.frombuffer(data, dtype=self._dtype)

    def set(self, data: Buffer, update: bool = False) -> None:
        """Set the data in the UI data, creating it when it is not set.

        Args:
            data (Buffer): Data to set. NumPy arrays and other buffer-protocol
                objects are accepted.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_set(self._value_id, data, update, None)

    def add(self, data: Buffer, update: bool = False) -> None:
        """Add the data to the UI data.

        Args:
            data (Buffer): Data to append. NumPy arrays and other
                buffer-protocol objects are accepted.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_add(self._value_id, data, update, None)

    def replace(self, data: Buffer, index: int, update: bool = False) -> None:
        """Replace the data in the UI data.

        Args:
            data (Buffer): Replacement data. NumPy arrays and other
                buffer-protocol objects are accepted.
            index (int): Index at which replacement starts.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_replace(self._value_id, data, index, update, None)

    def remove(self, index: int, count: int, update: bool = False) -> None:
        """Remove the data from the UI data.

        Args:
            index (int): Index at which removal starts.
            count (int): Number of items to remove.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_remove(self._value_id, index, count, update, None)

    def clear(self, update: bool = False) -> None:
        """Clear the data in the UI data; the data stays set, but empty.

        Args:
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_clear(self._value_id, update, None)

    def reset(self, update: bool = False) -> None:
        """Return the data, and its header, to "not set".

        Data that is not set stays so.

        Args:
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_reset(self._value_id, update)


class DataWithHeader[T: np.generic, H](Data[T]):
    """:class:`Data` with a typed header ``H`` published together with the buffer.

    The data never exists without a header: the :meth:`set` that creates the
    data must pass one, and :meth:`reset` drops both. After that, every change
    can carry a new header, and a change without one keeps the current header.
    The client receives the header with the buffer it belongs to, so it never
    shows a new header with an old buffer.

    Args:
        dtype (type[T]): One of the NumPy scalar types supported by
            :class:`Data`.
        header_id (int): Index of the header's protocol type in the generated
            type list.
    """

    def __init__(self, dtype: type[T], header_id: int) -> None:
        super().__init__(dtype)
        self._header_id = header_id

    def get_header(self) -> H | None:
        """Return the current header.

        Returns:
            H | None: The header, or ``None`` while the data is not set.
        """
        return self._server.data_get_header(self._value_id)

    def get_with_header(self) -> tuple[npt.NDArray[T], H] | None:
        """Return a snapshot of the buffer and the header read together.

        Separate :meth:`get` and :meth:`get_header` calls can each see a
        different change made in between.

        Returns:
            tuple[npt.NDArray[T], H] | None: A NumPy view backed by a copied
                ``bytearray`` and the header, or ``None`` while the data is
                not set.
        """
        entry = self._server.data_get_with_header(self._value_id)
        if entry is None:
            return None
        data, header = entry
        return np.frombuffer(data, dtype=self._dtype), header

    def set(self, data: Buffer, update: bool = False, header: H | _Unset = UNSET) -> None:
        """Set the data and optionally the header.

        Args:
            data (Buffer): Data to set.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
            header (H | _Unset, optional): New header; :data:`UNSET` keeps the
                current one. Defaults to :data:`UNSET`.

        Raises:
            ValueError: If the data is not set and no header is given.
        """
        self._server.data_set(self._value_id, data, update, _header(header))

    def add(self, data: Buffer, update: bool = False, header: H | _Unset = UNSET) -> None:
        """Append data and optionally replace the header.

        Args:
            data (Buffer): Data to append.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
            header (H | _Unset, optional): New header; :data:`UNSET` keeps the
                current one. Defaults to :data:`UNSET`.
        """
        self._server.data_add(self._value_id, data, update, _header(header))

    def replace(self, data: Buffer, index: int, update: bool = False, header: H | _Unset = UNSET) -> None:
        """Replace data from ``index`` and optionally the header.

        Args:
            data (Buffer): Replacement data.
            index (int): Index at which replacement starts.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
            header (H | _Unset, optional): New header; :data:`UNSET` keeps the
                current one. Defaults to :data:`UNSET`.
        """
        self._server.data_replace(self._value_id, data, index, update, _header(header))

    def remove(self, index: int, count: int, update: bool = False, header: H | _Unset = UNSET) -> None:
        """Remove data and optionally replace the header.

        Args:
            index (int): Index at which removal starts.
            count (int): Number of items to remove.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
            header (H | _Unset, optional): New header; :data:`UNSET` keeps the
                current one. Defaults to :data:`UNSET`.
        """
        self._server.data_remove(self._value_id, index, count, update, _header(header))

    def clear(self, update: bool = False, header: H | _Unset = UNSET) -> None:
        """Clear the data; the header stays unless a new one is given.

        Args:
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
            header (H | _Unset, optional): New header; :data:`UNSET` keeps the
                current one. Defaults to :data:`UNSET`.
        """
        self._server.data_clear(self._value_id, update, _header(header))

    def set_header(self, header: H, update: bool = False) -> None:
        """Replace the header alone.

        The client does not acknowledge a header, so this does not wait for
        the client itself: headers set faster than the client reads them
        queue up, as values do. It is queued after a buffer change that
        another thread is still sending, though, and waits as long as that
        change waits for the client.

        Args:
            header (H): New header.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.

        Raises:
            ValueError: If the data is not set.
        """
        self._server.data_set_header(self._value_id, header, update)


class DataTake[T: np.generic](_StaticBase):
    """One-shot numeric buffer sent to and consumed by the client.

    Methods that take a buffer read it until they return, while other Python
    threads keep running. Do not modify the buffer from another thread during
    the call, or the client may receive a mix of old and new contents; pass a
    copy if another thread may write to it.

    Args:
        dtype (type[T]): One of the integer or floating-point NumPy scalar
            types supported by :class:`Data`.
    """

    _header_id: int | None = None

    def __init__(self, dtype: type[T]) -> None:
        self._dtype = dtype

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        header = None if self._header_id is None else types[self._header_id]
        self._value_id = self._server.add_data_take(name, _get_dtype_id(self._dtype), header)

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
        self._server.data_take_set(self._value_id, data, blocking, update, cache, None)


class DataTakeWithHeader[T: np.generic, H](DataTake[T]):
    """:class:`DataTake` whose transfers carry a typed header ``H``.

    Every transfer carries a header.

    Args:
        dtype (type[T]): One of the NumPy scalar types supported by
            :class:`Data`.
        header_id (int): Index of the header's protocol type in the generated
            type list.
    """

    def __init__(self, dtype: type[T], header_id: int) -> None:
        super().__init__(dtype)
        self._header_id = header_id

    def set(
        self,
        data: Buffer,
        blocking: bool = False,
        update: bool = False,
        cache: bool = False,
        header: H | _Unset = UNSET,
    ) -> None:
        """Send data with its header.

        Args:
            data (Buffer): Data to send.
            blocking (bool, optional): Whether the next send waits until the
                client consumes this data. Defaults to False.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
            cache (bool, optional): Whether to cache the last sent data and
                header for a newly initialized client. Defaults to False.
            header (H | _Unset, optional): Header sent with the data; it is
                required, :data:`UNSET` raises. Defaults to :data:`UNSET`.

        Raises:
            ValueError: If no header is given.
        """
        self._server.data_take_set(self._value_id, data, blocking, update, cache, _header(header))


class SingleData[T: np.generic]:
    """Handle for one indexed buffer within a :class:`DataMulti` state.

    Instances are returned by :meth:`DataMulti.get` or indexed access; users
    do not normally construct them directly.

    Methods that take a buffer read it until they return, while other Python
    threads keep running. Do not modify the buffer from another thread during
    the call, or the client may receive a mix of old and new contents; pass a
    copy if another thread may write to it.
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
        self._server.data_multi_set(self._value_id, self._index, data, update, None)

    def add(self, data: Buffer, update: bool = False) -> None:
        """Add the data to the UI data at this index.

        Args:
            data (Buffer): Data to append. NumPy arrays and other
                buffer-protocol objects are accepted.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_multi_add(self._value_id, self._index, data, update, None)

    def replace(self, data: Buffer, index: int, update: bool = False) -> None:
        """Replace the data in the UI data at this index.

        Args:
            data (Buffer): Replacement data. NumPy arrays and other
                buffer-protocol objects are accepted.
            index (int): Index at which replacement starts within this buffer.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_multi_replace(self._value_id, self._index, data, index, update, None)

    def remove(self, index: int, count: int, update: bool = False) -> None:
        """Remove the data from the UI data at this index.

        Args:
            index (int): Index at which removal starts within this buffer.
            count (int): Number of items to remove.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_multi_remove(self._value_id, self._index, index, count, update, None)

    def clear(self, update: bool = False) -> None:
        """Clear the data in the UI data at this index.

        Args:
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_multi_clear(self._value_id, self._index, update, None)


class SingleDataWithHeader[T: np.generic, H](SingleData[T]):
    """:class:`SingleData` of a :class:`DataMultiWithHeader`, with its header.

    The :meth:`set` that creates the index must pass a header.
    """

    def get_header(self) -> H:
        """Return the header at this index.

        Returns:
            H: The header.

        Raises:
            ValueError: If the index does not exist.
        """
        return self._server.data_multi_get_header(self._value_id, self._index)

    def get_with_header(self) -> tuple[npt.NDArray[T], H]:
        """Return a snapshot of the buffer and its header read together.

        Separate :meth:`get` and :meth:`get_header` calls can each see a
        different change made in between.

        Returns:
            tuple[npt.NDArray[T], H]: A NumPy view backed by a copied
                ``bytearray``, and the header.

        Raises:
            ValueError: If the index does not exist.
        """
        data, header = self._server.data_multi_get_with_header(self._value_id, self._index)
        return np.frombuffer(data, dtype=self._dtype), header

    def set(self, data: Buffer, update: bool = False, header: H | _Unset = UNSET) -> None:
        """Set the data at this index and optionally its header.

        Args:
            data (Buffer): Data to set.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
            header (H | _Unset, optional): New header; :data:`UNSET` keeps the
                current one. Defaults to :data:`UNSET`.

        Raises:
            ValueError: If the index does not exist and no header is given.
        """
        self._server.data_multi_set(self._value_id, self._index, data, update, _header(header))

    def add(self, data: Buffer, update: bool = False, header: H | _Unset = UNSET) -> None:
        """Append data at this index and optionally replace its header.

        Args:
            data (Buffer): Data to append.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
            header (H | _Unset, optional): New header; :data:`UNSET` keeps the
                current one. Defaults to :data:`UNSET`.
        """
        self._server.data_multi_add(self._value_id, self._index, data, update, _header(header))

    def replace(self, data: Buffer, index: int, update: bool = False, header: H | _Unset = UNSET) -> None:
        """Replace data from ``index`` and optionally the header.

        Args:
            data (Buffer): Replacement data.
            index (int): Index at which replacement starts within this buffer.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
            header (H | _Unset, optional): New header; :data:`UNSET` keeps the
                current one. Defaults to :data:`UNSET`.
        """
        self._server.data_multi_replace(self._value_id, self._index, data, index, update, _header(header))

    def remove(self, index: int, count: int, update: bool = False, header: H | _Unset = UNSET) -> None:
        """Remove data at this index and optionally replace its header.

        Args:
            index (int): Index at which removal starts within this buffer.
            count (int): Number of items to remove.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
            header (H | _Unset, optional): New header; :data:`UNSET` keeps the
                current one. Defaults to :data:`UNSET`.
        """
        self._server.data_multi_remove(self._value_id, self._index, index, count, update, _header(header))

    def clear(self, update: bool = False, header: H | _Unset = UNSET) -> None:
        """Clear the data at this index; its header stays unless a new one is given.

        Args:
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
            header (H | _Unset, optional): New header; :data:`UNSET` keeps the
                current one. Defaults to :data:`UNSET`.
        """
        self._server.data_multi_clear(self._value_id, self._index, update, _header(header))

    def set_header(self, header: H, update: bool = False) -> None:
        """Replace the header at this index alone.

        It does not wait for the client itself, but it is queued after a
        buffer change to any index that another thread is still sending.

        Args:
            header (H): New header.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.

        Raises:
            ValueError: If the index does not exist.
        """
        self._server.data_multi_set_header(self._value_id, self._index, header, update)


class DataMulti[T: np.generic](_StaticBase):
    """Collection of numeric buffers indexed by non-negative integers.

    Args:
        dtype (type[T]): One of the integer or floating-point NumPy scalar
            types supported by :class:`Data`.
    """

    _header_id: int | None = None

    def __init__(self, dtype: type[T]) -> None:
        self._dtype = dtype

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        header = None if self._header_id is None else types[self._header_id]
        self._value_id = self._server.add_data_multi(name, _get_dtype_id(self._dtype), header)

    def get(self, index: int) -> SingleData[T]:
        """Get the SingleData object for the given index.

        Args:
            index (int): The index of the ``SingleData`` object.

        Returns:
            SingleData[T]: The ``SingleData`` object for the given index.
        """
        return SingleData(self._dtype, self._server, self._value_id, index)

    def remove_index(self, index: int, update: bool = False) -> None:
        """Remove the given index and its header from the DataMulti.

        Args:
            index (int): The index to remove.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_multi_remove_index(self._value_id, index, update)

    def reset(self, update: bool = False) -> None:
        """Reset (clear all indices and headers) in the DataMulti.

        Args:
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
        """
        self._server.data_multi_reset(self._value_id, update)

    def __getitem__(self, index: int) -> SingleData[T]:
        if isinstance(index, int):
            return self.get(index)
        raise TypeError("index must be an integer")


class DataMultiWithHeader[T: np.generic, H](DataMulti[T]):
    """:class:`DataMulti` whose buffers each carry a typed header ``H``.

    Args:
        dtype (type[T]): One of the NumPy scalar types supported by
            :class:`Data`.
        header_id (int): Index of the header's protocol type in the generated
            type list.
    """

    def __init__(self, dtype: type[T], header_id: int) -> None:
        super().__init__(dtype)
        self._header_id = header_id

    def get(self, index: int) -> SingleDataWithHeader[T, H]:
        """Get the SingleDataWithHeader object for the given index.

        Args:
            index (int): The index of the ``SingleDataWithHeader`` object.

        Returns:
            SingleDataWithHeader[T, H]: The handle for the given index.
        """
        return SingleDataWithHeader(self._dtype, self._server, self._value_id, index)

    def __getitem__(self, index: int) -> SingleDataWithHeader[T, H]:
        if isinstance(index, int):
            return self.get(index)
        raise TypeError("index must be an integer")


class SingleDataTake[T: np.generic]:
    """Handle for one indexed transfer within a :class:`DataMultiTake` state.

    Instances are returned by :meth:`DataMultiTake.get` or indexed access;
    users do not normally construct them directly.

    Methods that take a buffer read it until they return, while other Python
    threads keep running. Do not modify the buffer from another thread during
    the call, or the client may receive a mix of old and new contents; pass a
    copy if another thread may write to it.
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
                newly initialized client, which holds only cached indices.
                Defaults to False.
        """
        self._server.data_multi_take_set(
            self._value_id,
            self._index,
            data,
            blocking,
            update,
            cache,
            None,
        )


class SingleDataTakeWithHeader[T: np.generic, H](SingleDataTake[T]):
    """:class:`SingleDataTake` of a :class:`DataMultiTakeWithHeader`."""

    def set(
        self,
        data: Buffer,
        blocking: bool = False,
        update: bool = False,
        cache: bool = False,
        header: H | _Unset = UNSET,
    ) -> None:
        """Send data at this index with its header.

        Args:
            data (Buffer): Data to send.
            blocking (bool, optional): Whether the next send waits until the
                client consumes this data. Defaults to False.
            update (bool, optional): Whether to request a client repaint.
                Defaults to False.
            cache (bool, optional): Whether to cache the last sent data and
                header for a newly initialized client, which holds only cached
                indices. Defaults to False.
            header (H | _Unset, optional): Header sent with the data; it is
                required, :data:`UNSET` raises. Defaults to :data:`UNSET`.

        Raises:
            ValueError: If no header is given.
        """
        self._server.data_multi_take_set(
            self._value_id,
            self._index,
            data,
            blocking,
            update,
            cache,
            _header(header),
        )


class DataMultiTake[T: np.generic](_StaticBase):
    """Keyed one-shot numeric buffers sent to and consumed by the client.

    Args:
        dtype (type[T]): One of the integer or floating-point NumPy scalar
            types supported by :class:`Data`.
    """

    _header_id: int | None = None

    def __init__(self, dtype: type[T]) -> None:
        self._dtype = dtype

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        header = None if self._header_id is None else types[self._header_id]
        self._value_id = self._server.add_data_multi_take(
            name,
            _get_dtype_id(self._dtype),
            header,
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


class DataMultiTakeWithHeader[T: np.generic, H](DataMultiTake[T]):
    """:class:`DataMultiTake` whose transfers carry a typed header ``H``.

    Args:
        dtype (type[T]): One of the NumPy scalar types supported by
            :class:`Data`.
        header_id (int): Index of the header's protocol type in the generated
            type list.
    """

    def __init__(self, dtype: type[T], header_id: int) -> None:
        super().__init__(dtype)
        self._header_id = header_id

    def get(self, index: int) -> SingleDataTakeWithHeader[T, H]:
        """Get the SingleDataTakeWithHeader object for the given index.

        Args:
            index (int): The index of the ``SingleDataTakeWithHeader`` object.

        Returns:
            SingleDataTakeWithHeader[T, H]: The handle for the given index.
        """
        return SingleDataTakeWithHeader(self._dtype, self._server, self._value_id, index)

    def __getitem__(self, index: int) -> SingleDataTakeWithHeader[T, H]:
        if isinstance(index, int):
            return self.get(index)
        raise TypeError("index must be an integer")


_TRANSFER_OFFER = 0
_TRANSFER_FINISHED = 1


class _TransferEndpoint(_CallbackBase):
    """The callbacks every transfer endpoint has."""

    _meta_id: int | None = None
    _download: bool

    def __init__(self) -> None:
        self._finished: list[Callable[[TransferHandle], Any]] = []
        self._dispatching = False

    def _dispatch_event(self, event: tuple[int, int]) -> None:
        kind, key = event
        if kind == _TRANSFER_FINISHED:
            handle = self._server.transfer_finished(self._value_id, self._download, key)
            if handle is not None:
                for callback in list(self._finished):
                    self._signals_manager._invoke(callback, TransferHandle(handle))
        else:
            self._dispatch_offer()

    def _dispatch_offer(self) -> None:
        pass

    def _wants_events(self) -> bool:
        return bool(self._finished)

    def _update_dispatch(self) -> None:
        wanted = self._wants_events()
        if wanted and not self._dispatching:
            self._signals_manager.add_callback(self._value_id, self._dispatch_event)
        elif not wanted and self._dispatching:
            self._signals_manager.remove_callback(self._value_id, self._dispatch_event)
        self._dispatching = wanted

    def last(self) -> TransferHandle | None:
        """Return the most recent transfer of this endpoint.

        Returns:
            TransferHandle | None: The transfer, finished or not, or ``None``
                before the first one.
        """
        handle = self._server.transfer_last(self._value_id, self._download)
        return None if handle is None else TransferHandle(handle)

    def connect(self, callback: Callable[[TransferHandle], Any]) -> None:
        """Call ``callback`` once for every transfer when it is done or failed.

        The callback runs on a callback worker; exceptions go to the server's
        error handler.

        Args:
            callback (Callable[[TransferHandle], Any]): Receives the finished
                transfer.
        """
        self._finished.append(callback)
        self._update_dispatch()

    def disconnect(self, callback: Callable[[TransferHandle], Any]) -> None:
        """Remove a callback added with :meth:`connect`.

        Args:
            callback (Callable[[TransferHandle], Any]): Callback to remove.
        """
        if callback in self._finished:
            self._finished.remove(callback)
            self._update_dispatch()

    def disconnect_all(self) -> None:
        """Remove every callback added with :meth:`connect`."""
        self._finished.clear()
        self._update_dispatch()


class Download(_TransferEndpoint):
    """Endpoint that sends files from the server to the client.

    The client receives each offer and decides where the data goes; the data
    is read once the client accepts. The endpoint carries at most one
    unfinished transfer at a time: a send while it is busy raises, or waits
    with ``wait=True``. Do not wait inside a callback: it holds a callback
    worker for as long as it waits.
    """

    _download = True

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        meta = None if self._meta_id is None else types[self._meta_id]
        self._value_id = self._server.add_download(name, meta)

    @staticmethod
    def _meta(meta: object) -> tuple[object] | None:
        return None

    def send_file(
        self,
        path: str | os.PathLike[str],
        name: str | None = None,
        wait: bool = False,
        timeout: float | None = None,
    ) -> TransferHandle:
        """Offer one file to the client.

        The size and modification time are read now; the file itself is read
        once the client accepts, and a file that changed size by then fails
        the transfer.

        Args:
            path (str | os.PathLike[str]): The file to send.
            name (str | None, optional): The name the client sees; ``None``
                uses the file's own name. Defaults to None.
            wait (bool, optional): Whether to wait for the endpoint to become
                free instead of raising. Defaults to False.
            timeout (float | None, optional): Seconds to wait at most with
                ``wait``. Defaults to None.

        Returns:
            TransferHandle: The transfer, pending until the client accepts.

        Raises:
            TransferError: If the endpoint is busy or no client is connected.
            TimeoutError: If the wait ran out.
            OSError: If the file cannot be read.
            ValueError: If the name or metadata is invalid, or ``timeout`` is
                given without ``wait``.
        """
        return self._send_file(path, name, None, wait, timeout)

    def _send_file(
        self,
        path: str | os.PathLike[str],
        name: str | None,
        meta: object,
        wait: bool,
        timeout: float | None,
    ) -> TransferHandle:
        handle = self._server.transfer_send_file(self._value_id, os.fspath(path), name, self._meta(meta), wait, timeout)
        return TransferHandle(handle)

    def send_files(
        self,
        paths: Sequence[str | os.PathLike[str]],
        name: str,
        root: str | os.PathLike[str] | None = None,
        wait: bool = False,
        timeout: float | None = None,
    ) -> TransferHandle:
        """Offer several files to the client as one transfer.

        Each file is offered under its own name or, with ``root``, under its
        path relative to ``root``, so the client recreates the folders. To
        send a folder, pass the files below it and the folder as ``root``.

        Args:
            paths (Sequence[str | os.PathLike[str]]): The files to send.
            name (str): The name of the set.
            root (str | os.PathLike[str] | None, optional): The folder the
                names are relative to. Defaults to None.
            wait (bool, optional): Whether to wait for the endpoint to become
                free instead of raising. Defaults to False.
            timeout (float | None, optional): Seconds to wait at most with
                ``wait``. Defaults to None.

        Returns:
            TransferHandle: The transfer, pending until the client accepts.

        Raises:
            TransferError: If the endpoint is busy or no client is connected.
            TimeoutError: If the wait ran out.
            OSError: If a file cannot be read.
            ValueError: If there are no files, a file lies outside ``root``,
                two files have the same name, or ``timeout`` is given without
                ``wait``.
        """
        return self._send_files(paths, name, root, None, wait, timeout)

    def _send_files(
        self,
        paths: Sequence[str | os.PathLike[str]],
        name: str,
        root: str | os.PathLike[str] | None,
        meta: object,
        wait: bool,
        timeout: float | None,
    ) -> TransferHandle:
        handle = self._server.transfer_send_files(
            self._value_id,
            [os.fspath(path) for path in paths],
            name,
            None if root is None else os.fspath(root),
            self._meta(meta),
            wait,
            timeout,
        )
        return TransferHandle(handle)

    def send_bytes(
        self,
        data: Buffer,
        name: str,
        wait: bool = False,
        timeout: float | None = None,
    ) -> TransferHandle:
        """Offer a buffer to the client as one file.

        Args:
            data (Buffer): The data; ``bytes``, ``bytearray``, ``memoryview``
                or a NumPy array. It is copied once.
            name (str): The file name the client sees.
            wait (bool, optional): Whether to wait for the endpoint to become
                free instead of raising. Defaults to False.
            timeout (float | None, optional): Seconds to wait at most with
                ``wait``. Defaults to None.

        Returns:
            TransferHandle: The transfer, pending until the client accepts.

        Raises:
            TransferError: If the endpoint is busy or no client is connected.
            TimeoutError: If the wait ran out.
            ValueError: If the name or metadata is invalid, or ``timeout`` is
                given without ``wait``.
        """
        return self._send_bytes(data, name, None, wait, timeout)

    def _send_bytes(
        self,
        data: Buffer,
        name: str,
        meta: object,
        wait: bool,
        timeout: float | None,
    ) -> TransferHandle:
        handle = self._server.transfer_send_bytes(self._value_id, data, name, self._meta(meta), wait, timeout)
        return TransferHandle(handle)


class DownloadWithMeta[M](Download):
    """:class:`Download` whose offers carry typed metadata ``M``.

    Every send takes the metadata as the required keyword ``meta``.

    Args:
        meta_id (int): Index of the metadata's protocol type in the generated
            type list.
    """

    def __init__(self, meta_id: int) -> None:
        super().__init__()
        self._meta_id = meta_id

    @staticmethod
    def _meta(meta: object) -> tuple[object] | None:
        return (meta,)

    def send_file(  # type: ignore[override]
        self,
        path: str | os.PathLike[str],
        name: str | None = None,
        *,
        meta: M,
        wait: bool = False,
        timeout: float | None = None,
    ) -> TransferHandle:
        """Offer one file to the client; see :meth:`Download.send_file`.

        Args:
            path (str | os.PathLike[str]): The file to send.
            name (str | None, optional): The name the client sees; ``None``
                uses the file's own name. Defaults to None.
            meta (M): The metadata sent with the offer.
            wait (bool, optional): Whether to wait for the endpoint to become
                free instead of raising. Defaults to False.
            timeout (float | None, optional): Seconds to wait at most with
                ``wait``. Defaults to None.

        Returns:
            TransferHandle: The transfer, pending until the client accepts.
        """
        return self._send_file(path, name, meta, wait, timeout)

    def send_files(  # type: ignore[override]
        self,
        paths: Sequence[str | os.PathLike[str]],
        name: str,
        root: str | os.PathLike[str] | None = None,
        *,
        meta: M,
        wait: bool = False,
        timeout: float | None = None,
    ) -> TransferHandle:
        """Offer several files as one transfer; see :meth:`Download.send_files`.

        Args:
            paths (Sequence[str | os.PathLike[str]]): The files to send.
            name (str): The name of the set.
            root (str | os.PathLike[str] | None, optional): The folder the
                names are relative to. Defaults to None.
            meta (M): The metadata sent with the offer.
            wait (bool, optional): Whether to wait for the endpoint to become
                free instead of raising. Defaults to False.
            timeout (float | None, optional): Seconds to wait at most with
                ``wait``. Defaults to None.

        Returns:
            TransferHandle: The transfer, pending until the client accepts.
        """
        return self._send_files(paths, name, root, meta, wait, timeout)

    def send_bytes(  # type: ignore[override]
        self,
        data: Buffer,
        name: str,
        *,
        meta: M,
        wait: bool = False,
        timeout: float | None = None,
    ) -> TransferHandle:
        """Offer a buffer as one file; see :meth:`Download.send_bytes`.

        Args:
            data (Buffer): The data; it is copied once.
            name (str): The file name the client sees.
            meta (M): The metadata sent with the offer.
            wait (bool, optional): Whether to wait for the endpoint to become
                free instead of raising. Defaults to False.
            timeout (float | None, optional): Seconds to wait at most with
                ``wait``. Defaults to None.

        Returns:
            TransferHandle: The transfer, pending until the client accepts.
        """
        return self._send_bytes(data, name, meta, wait, timeout)


class Upload(_TransferEndpoint):
    """Endpoint that receives files from the client.

    The server receives each offer and decides where the data goes. The
    endpoint carries at most one unfinished transfer at a time.
    """

    _download = False

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        meta = None if self._meta_id is None else types[self._meta_id]
        self._value_id = self._server.add_upload(name, meta)


class UploadWithMeta[M](Upload):
    """:class:`Upload` whose offers carry typed metadata ``M``.

    Args:
        meta_id (int): Index of the metadata's protocol type in the generated
            type list.
    """

    def __init__(self, meta_id: int) -> None:
        super().__init__()
        self._meta_id = meta_id


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
    "UNSET",
]
