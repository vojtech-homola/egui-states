# ruff: noqa: D107 D105 D102 PLC2801
"""Typed state handles used by generated Python server bindings."""

from __future__ import annotations

import operator
import os
from abc import ABC, abstractmethod
from collections.abc import Buffer, Callable, Iterator, Sequence
from typing import Any, Final, SupportsIndex

import numpy as np
import numpy.typing as npt

from egui_states import _core
from egui_states._core import (
    PyObjectType,
    StateServerCore,
    bo,
    cl,
    denu,
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
    vnamed,
    vtuple,
    vunit,
)
from egui_states.signals import SignalsManager
from egui_states.transfer import TransferHandle, TransferOffer

#: Gray, gray-alpha, RGB, or RGBA color whose components are in ``0..255``.
type ImageColor = int | tuple[int, int] | tuple[int, int, int] | tuple[int, int, int, int]


class _CustomStruct:
    __getitem__ = object.__getattribute__


class _CustomEnum:
    """Base class of a generated enum with data.

    The class is only a namespace. Each variant is a frozen dataclass nested
    in it, created as ``Command.Stop()``, ``Command.MoveTo(x=1.0, y=2.0)`` or
    ``Command.SetGain(0.5)``, and ``Command.Type`` is the union of the
    variants: the type of a value.
    """

    __slots__ = ()

    def __init__(self) -> None:
        name = type(self).__qualname__
        raise TypeError(f"{name} is a namespace of variants, not a value; create one of its variants")


class _TupleVariant:
    """Base class of a generated tuple variant, whose fields are ``_0``, ``_1``, ..."""

    __slots__ = ()
    __match_args__: tuple[str, ...]

    def __repr__(self) -> str:
        fields = ", ".join(repr(getattr(self, field)) for field in self.__match_args__)
        return f"{type(self).__qualname__}({fields})"


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
        The state definition sets the starting mode (``#[state(queue)]``); this
        overrides it until the next switch. Changes already pending are kept.
        """
        self._server.signal_set_to_queue(self._value_id)

    def signal_set_to_single(self) -> None:
        """Coalesce pending signaled changes to the latest value.

        The state definition sets the starting mode, single unless the field
        is marked ``#[state(queue)]``; this overrides it until the next switch.
        Of the changes already queued, only the newest is delivered.
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

    def set(self, value: T, set_signal: bool = False) -> None:
        """Replace the value and send it to the client.

        Args:
            value (T): Value to store locally and send to the client.
            set_signal (bool, optional): Whether to emit callbacks for the new
                value. Defaults to False.
        """
        self._server.value_set(self._value_id, value, set_signal)

    def get(self) -> T:
        """Return the server's current value.

        Returns:
            T: The current value.
        """
        return self._server.value_get(self._value_id)

    def connect(self, callback: Callable[[T], Any]) -> Callable[[T], Any]:
        """Connect a callback to the value.

        Args:
            callback (Callable[[T], Any]): The callback to connect.

        Returns:
            Callable[[T], Any]: ``callback``, so the method also works as
                a decorator.
        """
        self._signals_manager.add_callback(self._value_id, callback)
        return callback

    def connect_previous(self, callback: Callable[[T, T], Any]) -> Callable[[T, T], Any]:
        """Connect a callback which also receives the previous value.

        The previous value is the one the callback was last notified about, not
        necessarily the immediate predecessor: in single mode successive changes
        coalesce, so a -> b -> c delivering only c calls the callback with
        (c, a).

        Args:
            callback (Callable[[T, T], Any]): The callback to connect. It is
                called with the new value followed by the previous one.

        Returns:
            Callable[[T, T], Any]: ``callback``, so the method also works as
                a decorator.
        """
        self._signals_manager.add_callback_previous(self._value_id, callback)
        return callback

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

    def set(self, value: T, blocking: bool = False) -> None:
        """Send a one-shot value to the client.

        Args:
            value (T): The value to set.
            blocking (bool, optional): Whether the next send waits until the
                client consumes this value. Defaults to False.
        """
        self._server.value_take_set(self._value_id, value, blocking)


class ValueTakeEmpty(_StaticBase):
    """Unit-valued one-shot event sent to and consumed once by the client."""

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_value_take(name, emp)

    def set(self, blocking: bool = False) -> None:
        """Set the value of the UI element.

        Args:
            blocking (bool, optional): Whether the next send waits until the
                client consumes this value. Defaults to False.
        """
        self._server.value_take_set(self._value_id, (), blocking)


class Static[T](_StaticBase):
    """Server-controlled value that the egui client can read but not modify."""

    def __init__(self, obj_id: int, initial_value: T) -> None:
        self._initial_value = initial_value
        self._obj_id = obj_id

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_static(name, types[self._obj_id], self._initial_value)
        del self._initial_value
        del self._obj_id

    def set(self, value: T) -> None:
        """Replace the value mirrored to the client.

        Args:
            value (T): The value to set.
        """
        self._server.static_set(self._value_id, value)

    def get(self) -> T:
        """Get the static value of the UI.

        Returns:
            T: The static value.
        """
        return self._server.static_get(self._value_id)


class Signal[T](_SignalBase):
    """Client-to-server event that does not retain a client-side value.

    Calling :meth:`emit` on the server runs the local callbacks, as if the
    client had emitted the event.
    """

    def __init__(self, obj_id: int, queue: bool = False) -> None:
        self._obj_id = obj_id
        self._queue = queue

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_signal(name, types[self._obj_id], self._queue)
        del self._obj_id
        del self._queue

    def emit(self, value: T) -> None:
        """Emit the event to the connected callbacks.

        Nothing is stored and nothing is sent to the client.

        Args:
            value (T): The event's value.
        """
        self._server.signal_set(self._value_id, value)

    def connect(self, callback: Callable[[T], Any]) -> Callable[[T], Any]:
        """Connect a callback to the signal.

        Args:
            callback (Callable[[T], Any]): Callback receiving the signal value.

        Returns:
            Callable[[T], Any]: ``callback``, so the method also works as
                a decorator.
        """
        self._signals_manager.add_callback(self._value_id, callback)
        return callback

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

    def emit(self) -> None:
        """Emit the event to the connected callbacks.

        Nothing is sent to the client.
        """
        self._server.signal_set(self._value_id, ())

    def connect[F: Callable[[], Any]](self, callback: F) -> F:
        """Connect a callback to the signal.

        Args:
            callback (Callable[[], Any]): The callback to connect.

        Returns:
            Callable[[], Any]: ``callback``, so the method also works as
                a decorator.
        """
        self._signals_manager.add_callback(self._value_id, callback)
        return callback

    def disconnect(self, callback: Callable[[], Any]) -> None:
        """Disconnect a callback from the value.

        Args:
            callback (Callable[[], Any]): The callback to disconnect.
        """
        self._signals_manager.remove_callback(self._value_id, callback)

    def disconnect_all(self) -> None:
        """Disconnect all callbacks from the signal."""
        self._signals_manager.clear_callbacks(self._value_id)


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


#: Largest index of a multi state, which the protocol sends as a ``u32``.
_MAX_INDEX: Final = 0xFFFF_FFFF


def _index(index: object) -> int:
    """Return the index of a multi state as an ``int``.

    An index is whatever :func:`operator.index` accepts: an ``int``, a
    ``bool`` or a NumPy integer, but not a ``float`` or a ``str``.

    Args:
        index (object): The index to convert.

    Returns:
        int: The index, not checked against the ``u32`` range.

    Raises:
        TypeError: If ``index`` is not an integer.
    """
    if isinstance(index, SupportsIndex):
        try:
            return operator.index(index)
        except TypeError:
            # ``__index__`` alone does not make an index: an array of floats
            # has the method, and it raises.
            pass
    raise TypeError("index must be an integer")


def _image_array(data: bytearray, shape: tuple[int, int], channels: int) -> npt.NDArray[np.uint8]:
    """Return stored image bytes as an array shaped like an image to set."""
    array = np.frombuffer(data, dtype=np.uint8)
    if channels == 1:
        return array.reshape(shape)
    return array.reshape((shape[0], shape[1], channels))


class Image(_StaticBase):
    """Server-controlled image mirrored to an initialized client texture.

    The server keeps the image in the format of the last :meth:`set` or
    :meth:`set_all`: gray, gray-alpha, RGB or RGBA. An :meth:`update` with
    channels the image lacks widens it first, to the smallest format that
    holds both: gray gains color channels equal to it, and a missing alpha
    becomes 255, which is what the client shows. Until it is first set, the
    image has no format and :meth:`get` returns ``None``.

    Each dimension is at most 65,536 pixels, inclusive. Oversized images raise
    ``ValueError``. Client renderer and memory limits may be smaller.

    Methods that take a buffer read it until they return, while other Python
    threads keep running. Do not modify the buffer from another thread during
    the call, or the client may receive a mix of old and new contents; pass a
    copy if another thread may write to it.

    Args:
        low_priority (bool, optional): Whether the server sends its transfers
            in the low-priority lane: only while no other state has a message
            waiting, and a window of their data at a time, so small states do
            not wait behind them. A change of another state made after a
            change of this one can then reach the client first. Defaults to
            False.
    """

    _header_id: int | None = None

    def __init__(self, low_priority: bool = False) -> None:
        self._low_priority = low_priority

    def _header_type(self, types: list[PyObjectType]) -> PyObjectType | None:
        return None if self._header_id is None else types[self._header_id]

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_image(name, self._header_type(types), self._low_priority)

    def set(self, image: Buffer, wait: bool = False) -> None:
        """Set the image in the UI image.

        The client shows the image once all of it has arrived. One transfer
        of this image is sent at a time: the call blocks until the client has
        acknowledged the previous one, then queues the image and returns.
        Parts of a split :meth:`update` that are not sent yet are dropped
        first, because this image overwrites them.

        Args:
            image (Buffer): The image to set, shaped as ``(height, width)``
                for gray or ``(height, width, channels)`` with 2, 3 or 4
                channels. The server keeps it in this format.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this image, or the connection has ended.
                Defaults to False.
        """
        self._server.image_set(self._value_id, image, None, wait)

    def update(
        self,
        image: Buffer,
        origin: list[int] | tuple[int, int],
        force: bool = False,
        max_lines: int | None = None,
        wait: bool = False,
    ) -> None:
        """Update a rectangular part of the image.

        Like :meth:`set`, the call blocks until the client has acknowledged
        the previous transfer, then queues the update and returns. With
        ``max_lines``, only the first part is queued; each further part is
        sent once the client has acknowledged the one before, so later
        changes of other states can reach the client first, and the next call
        on this image waits until the last part has been acknowledged.

        When the rectangle has channels the stored image lacks, the server
        first widens the whole image to a format that holds both.

        Args:
            image (Buffer): The image rectangle to write.
            origin (list[int] | tuple[int, int]): Top-left origin as ``(y, x)``.
            force (bool, optional): Whether to first drop the parts of the
                previous split update that are not sent yet, if this rectangle
                covers its rectangle. The part being sent always completes.
                Defaults to False.
            max_lines (int | None, optional): Send the rectangle as parts of at
                most this many rows, each applied by the client as it arrives.
                ``None`` applies the whole rectangle at once. Defaults to None.
            wait (bool, optional): Whether to return only once the client has
                acknowledged the last part of this update, or the connection
                has ended. Defaults to False.

        Raises:
            ValueError: If ``max_lines`` is less than 1, or the rectangle is
                invalid.
        """
        self._server.image_update(self._value_id, image, origin, None, force, max_lines, wait)

    def set_all(
        self,
        shape: list[int] | tuple[int, int],
        color: ImageColor,
        wait: bool = False,
    ) -> None:
        """Fill the complete image with one color using a compact message.

        It waits for the previous transfer like :meth:`set`.

        Args:
            shape (list[int] | tuple[int, int]): Image shape as ``(height,
                width)``.
            color (ImageColor): Gray, gray-alpha, RGB, or RGBA color. Every
                component must be in ``0..255``. The image is stored in the
                format of the color: an integer stores gray, and a tuple of
                2, 3 or 4 components gray-alpha, RGB or RGBA.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this image, or the connection has ended.
                Defaults to False.
        """
        self._server.image_set_all(self._value_id, shape, color, None, wait)

    def is_idle(self) -> bool:
        """Return whether nothing is being sent and nothing waits.

        The next :meth:`set`, :meth:`set_all` or :meth:`update` is then sent
        without waiting. It is also true while no client is connected,
        because a change is then only stored. A producer that wants the
        freshest image on a slow link can skip a frame while this is false.

        Returns:
            bool: Whether the image is idle.
        """
        return self._server.image_is_idle(self._value_id)

    def get(self) -> npt.NDArray[np.uint8] | None:
        """Get the image in the format it is stored in.

        Returns:
            npt.NDArray[np.uint8] | None: The image, with shape ``(height,
            width)`` for gray and ``(height, width, channels)`` otherwise, as
            :meth:`set` takes it; ``None`` until the image is set.
        """
        image = self._server.image_get(self._value_id)
        if image is None:
            return None
        data, shape, channels = image
        return _image_array(data, shape, channels)

    def shape(self) -> tuple[int, int]:
        """Get the shape of the image.

        Returns:
            tuple[int, int]: Image shape as ``(height, width)``.
        """
        return self._server.image_size(self._value_id)


class ImageWithHeader[H](Image):
    """:class:`Image` with a typed header ``H`` published together with the image.

    The image never exists without a header: the :meth:`set` or
    :meth:`set_all` that creates it must pass one. After that, every change
    can carry a new header, and a change without one keeps the current
    header. The client stores the header with the pixels it belongs to. It
    uploads a texture at the end of the frame being drawn, so for that one
    frame the UI may still show the old pixels next to a new header.

    Args:
        header_id (int): Index of the header's protocol type in the generated
            type list.
        low_priority (bool, optional): Whether the server sends its transfers
            in the low-priority lane, as for :class:`Image`. Defaults to
            False.
    """

    def __init__(self, header_id: int, low_priority: bool = False) -> None:
        super().__init__(low_priority)
        self._header_id = header_id

    def get_header(self) -> H | None:
        """Return the current header.

        Returns:
            H | None: The header, or ``None`` until the image is set.
        """
        return self._server.image_get_header(self._value_id)

    def get_with_header(self) -> tuple[npt.NDArray[np.uint8], H] | None:
        """Return the image and its header, read together.

        Separate :meth:`get` and :meth:`get_header` calls can each see a
        different change made in between.

        Returns:
            tuple[npt.NDArray[np.uint8], H] | None: The image as :meth:`get`
                returns it and the header, or ``None`` until the image is set.
        """
        entry = self._server.image_get_with_header(self._value_id)
        if entry is None:
            return None
        (data, shape, channels), header = entry
        return _image_array(data, shape, channels), header

    def set(self, image: Buffer, wait: bool = False, header: H | _Unset = UNSET) -> None:
        """Set the image and optionally the header.

        It works as :meth:`Image.set`; the client shows the header with the
        image.

        Args:
            image (Buffer): The image to set, as for :meth:`Image.set`.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this image, or the connection has ended.
                Defaults to False.
            header (H | _Unset, optional): New header. Defaults to
                :data:`UNSET`, which keeps the current one.

        Raises:
            ValueError: If the image is not set yet and no header is given.
        """
        self._server.image_set(self._value_id, image, _header(header), wait)

    def update(
        self,
        image: Buffer,
        origin: list[int] | tuple[int, int],
        force: bool = False,
        max_lines: int | None = None,
        wait: bool = False,
        header: H | _Unset = UNSET,
    ) -> None:
        """Update a rectangular part of the image and optionally the header.

        It works as :meth:`Image.update`. With ``max_lines``, the header goes
        with the first part, and the client shows it together with that
        part; the later parts keep it.

        Args:
            image (Buffer): The image rectangle to write.
            origin (list[int] | tuple[int, int]): Top-left origin as ``(y, x)``.
            force (bool, optional): As for :meth:`Image.update`. Defaults to
                False.
            max_lines (int | None, optional): As for :meth:`Image.update`.
                Defaults to None.
            wait (bool, optional): As for :meth:`Image.update`. Defaults to
                False.
            header (H | _Unset, optional): New header. Defaults to
                :data:`UNSET`, which keeps the current one.

        Raises:
            ValueError: If ``max_lines`` is less than 1, or the rectangle is
                invalid.
        """
        self._server.image_update(self._value_id, image, origin, _header(header), force, max_lines, wait)

    def set_all(
        self,
        shape: list[int] | tuple[int, int],
        color: ImageColor,
        wait: bool = False,
        header: H | _Unset = UNSET,
    ) -> None:
        """Fill the complete image with one color and optionally set the header.

        Args:
            shape (list[int] | tuple[int, int]): Image shape as ``(height,
                width)``.
            color (ImageColor): As for :meth:`Image.set_all`.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this image, or the connection has ended.
                Defaults to False.
            header (H | _Unset, optional): New header. Defaults to
                :data:`UNSET`, which keeps the current one.

        Raises:
            ValueError: If the image is not set yet and no header is given.
        """
        self._server.image_set_all(self._value_id, shape, color, _header(header), wait)

    def set_header(self, header: H) -> None:
        """Replace the header alone.

        The client does not acknowledge a header, so this does not wait for
        the client itself. It is sent after the transfers of this image that
        are already queued, including the parts of a split :meth:`update` not
        sent yet, and waits until the client has acknowledged them.

        Args:
            header (H): New header.

        Raises:
            ValueError: If the image is not set.
        """
        self._server.image_set_header(self._value_id, header)


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

    def is_idle(self) -> bool:
        """Return whether nothing of this index is being sent and nothing waits.

        See :meth:`ImageMulti.is_idle`.

        Returns:
            bool: Whether this index is idle.
        """
        return self._server.image_multi_is_idle(self._value_id, self._index)

    def set(self, image: Buffer, wait: bool = False) -> None:
        """Create or replace this indexed image.

        Each index has its own transfer, so the call waits for the previous
        transfer of this index only, as :meth:`Image.set` does. Only parts of
        a split update of this index that are not sent yet are dropped.

        Args:
            image (Buffer): Image data shaped as ``(height, width)`` or
                ``(height, width, channels)``.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this image, or the connection has ended.
                Defaults to False.
        """
        self._server.image_multi_set(self._value_id, self._index, image, None, wait)

    def update(
        self,
        image: Buffer,
        origin: list[int] | tuple[int, int],
        force: bool = False,
        max_lines: int | None = None,
        wait: bool = False,
    ) -> None:
        """Update a rectangular part of this existing indexed image.

        It works as :meth:`Image.update`, for this index on its own:
        ``force`` drops only parts of this index, never parts of other
        indices, and the call waits only for transfers of this index.

        Args:
            image (Buffer): The image rectangle to write.
            origin (list[int] | tuple[int, int]): Top-left origin as ``(y, x)``.
            force (bool, optional): Whether to first drop the parts of the
                previous split update of this index that are not sent yet, if
                this rectangle covers its rectangle. The part being sent
                always completes. Defaults to False.
            max_lines (int | None, optional): Send the rectangle as parts of at
                most this many rows, each applied by the client as it arrives.
                ``None`` applies the whole rectangle at once. Defaults to None.
            wait (bool, optional): Whether to return only once the client has
                acknowledged the last part of this update, or the connection
                has ended. Defaults to False.

        Raises:
            ValueError: If ``max_lines`` is less than 1, or the rectangle is
                invalid.
        """
        self._server.image_multi_update(self._value_id, self._index, image, origin, None, force, max_lines, wait)

    def set_all(
        self,
        shape: list[int] | tuple[int, int],
        color: ImageColor,
        wait: bool = False,
    ) -> None:
        """Create or replace this indexed image with one color.

        It waits for the previous transfer like :meth:`set`.

        Args:
            shape (list[int] | tuple[int, int]): Image shape as ``(height,
                width)``.
            color (ImageColor): Gray, gray-alpha, RGB, or RGBA color. The
                image is stored in its format, as in :meth:`Image.set_all`.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this image, or the connection has ended.
                Defaults to False.
        """
        self._server.image_multi_set_all(self._value_id, self._index, shape, color, None, wait)

    def get(self) -> npt.NDArray[np.uint8]:
        """Return this indexed image in the format it is stored in.

        Returns:
            npt.NDArray[np.uint8]: The image, with shape ``(height, width)``
            for gray and ``(height, width, channels)`` otherwise, as
            :meth:`set` takes it.
        """
        data, shape, channels = self._server.image_multi_get(self._value_id, self._index)
        return _image_array(data, shape, channels)

    def shape(self) -> tuple[int, int]:
        """Return this indexed image shape.

        Returns:
            tuple[int, int]: Image shape as ``(height, width)``.
        """
        return self._server.image_multi_size(self._value_id, self._index)


class SingleImageWithHeader[H](SingleImage):
    """:class:`SingleImage` of an :class:`ImageMultiWithHeader`, with its header ``H``.

    Creating this index needs a header, as for :class:`ImageWithHeader`.
    """

    def get_header(self) -> H:
        """Return the header of this indexed image.

        Returns:
            H: The header.

        Raises:
            ValueError: If the index does not exist.
        """
        return self._server.image_multi_get_header(self._value_id, self._index)

    def get_with_header(self) -> tuple[npt.NDArray[np.uint8], H]:
        """Return this indexed image and its header, read together.

        Returns:
            tuple[npt.NDArray[np.uint8], H]: The image as :meth:`get` returns
                it and the header.

        Raises:
            ValueError: If the index does not exist.
        """
        (data, shape, channels), header = self._server.image_multi_get_with_header(self._value_id, self._index)
        return _image_array(data, shape, channels), header

    def set(self, image: Buffer, wait: bool = False, header: H | _Unset = UNSET) -> None:
        """Create or replace this indexed image, and optionally its header.

        Args:
            image (Buffer): Image data shaped as ``(height, width)`` or
                ``(height, width, channels)``.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this image, or the connection has ended.
                Defaults to False.
            header (H | _Unset, optional): New header. Defaults to
                :data:`UNSET`, which keeps the current one.

        Raises:
            ValueError: If the index does not exist yet and no header is
                given.
        """
        self._server.image_multi_set(self._value_id, self._index, image, _header(header), wait)

    def update(
        self,
        image: Buffer,
        origin: list[int] | tuple[int, int],
        force: bool = False,
        max_lines: int | None = None,
        wait: bool = False,
        header: H | _Unset = UNSET,
    ) -> None:
        """Update a rectangular part of this indexed image, and optionally its header.

        It works as :meth:`ImageWithHeader.update`, for this index on its own.

        Args:
            image (Buffer): The image rectangle to write.
            origin (list[int] | tuple[int, int]): Top-left origin as ``(y, x)``.
            force (bool, optional): As for :meth:`SingleImage.update`.
                Defaults to False.
            max_lines (int | None, optional): As for :meth:`SingleImage.update`.
                Defaults to None.
            wait (bool, optional): As for :meth:`SingleImage.update`. Defaults
                to False.
            header (H | _Unset, optional): New header. Defaults to
                :data:`UNSET`, which keeps the current one.
        """
        self._server.image_multi_update(
            self._value_id, self._index, image, origin, _header(header), force, max_lines, wait
        )

    def set_all(
        self,
        shape: list[int] | tuple[int, int],
        color: ImageColor,
        wait: bool = False,
        header: H | _Unset = UNSET,
    ) -> None:
        """Create or replace this indexed image with one color, and optionally its header.

        Args:
            shape (list[int] | tuple[int, int]): Image shape as ``(height,
                width)``.
            color (ImageColor): As for :meth:`Image.set_all`.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this image, or the connection has ended.
                Defaults to False.
            header (H | _Unset, optional): New header. Defaults to
                :data:`UNSET`, which keeps the current one.
        """
        self._server.image_multi_set_all(self._value_id, self._index, shape, color, _header(header), wait)

    def set_header(self, header: H) -> None:
        """Replace the header of this indexed image alone.

        It is sent and waits as :meth:`ImageWithHeader.set_header` does.

        Args:
            header (H): New header.

        Raises:
            ValueError: If the index does not exist.
        """
        self._server.image_multi_set_header(self._value_id, self._index, header)


class ImageMulti(_StaticBase):
    """Sparse collection of server-controlled images indexed by integers.

    Each image keeps its own format, as described at :class:`Image`.

    ``images[index]`` returns the handle of an index, whether it is populated
    or not. ``index in images``, ``len(images)`` and iteration cover the
    populated indices only; iteration walks a copy taken when it starts.
    :meth:`values` and :meth:`items` return the handles of those indices, as
    for a dict.

    Each image dimension is limited to 65,536 pixels, inclusive. Client renderer
    and memory limits may be smaller.

    Args:
        low_priority (bool, optional): Whether the server sends its transfers
            in the low-priority lane: only while no other state has a message
            waiting, and a window of their data at a time, so small states do
            not wait behind them. A change of another state made after a
            change of this one can then reach the client first. Defaults to
            False.
    """

    _header_id: int | None = None

    def __init__(self, low_priority: bool = False) -> None:
        self._low_priority = low_priority

    def _header_type(self, types: list[PyObjectType]) -> PyObjectType | None:
        return None if self._header_id is None else types[self._header_id]

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_image_multi(name, self._header_type(types), self._low_priority)

    def get(self, index: int) -> SingleImage:
        """Return a handle for one image index.

        Args:
            index (int): Non-negative image index.

        Returns:
            SingleImage: Handle that can create, or read ``index``.

        Raises:
            TypeError: If ``index`` is not an integer.
        """
        return SingleImage(self._server, self._value_id, _index(index))

    def is_idle(self, index: int | None = None) -> bool:
        """Return whether nothing is being sent and nothing waits.

        See :meth:`Image.is_idle`.

        Args:
            index (int | None, optional): The index to ask about. Defaults to
                None, which asks about every index.

        Returns:
            bool: Whether the index, or every index, is idle.
        """
        return self._server.image_multi_is_idle(self._value_id, index)

    def remove_index(self, index: int) -> None:
        """Remove an image index; an absent index is a no-op.

        Parts of a split update of this index that are not sent yet are
        dropped; the removal is queued once nothing is being sent.

        Args:
            index (int): Non-negative image index.
        """
        self._server.image_multi_remove_index(self._value_id, index)

    def reset(self) -> None:
        """Remove every populated image index."""
        self._server.image_multi_reset(self._value_id)

    def indices(self) -> list[int]:
        """Return populated indices.

        Returns:
            list[int]: Populated indices in ascending order.
        """
        return self._server.image_multi_indices(self._value_id)

    def __len__(self) -> int:
        return self._server.image_multi_len(self._value_id)

    def __contains__(self, index: object) -> bool:
        try:
            index = _index(index)
        except TypeError:
            return False
        return 0 <= index <= _MAX_INDEX and self._server.image_multi_contains(self._value_id, index)

    def __getitem__(self, index: int) -> SingleImage:
        return self.get(index)

    def __iter__(self) -> Iterator[int]:
        """Iterate over a copy of the populated indices."""
        return iter(self.indices())

    def values(self) -> Sequence[SingleImage]:
        """Return the handles of the populated indices.

        Returns:
            Sequence[SingleImage]: The handles, in ascending order of their
                indices.
        """
        return [self.get(index) for index in self.indices()]

    def items(self) -> Sequence[tuple[int, SingleImage]]:
        """Return the populated indices with their handles.

        Returns:
            Sequence[tuple[int, SingleImage]]: Pairs of an index and its handle,
                in ascending order of the indices.
        """
        return [(index, self.get(index)) for index in self.indices()]


class ImageMultiWithHeader[H](ImageMulti):
    """:class:`ImageMulti` whose images each carry a typed header ``H``.

    Args:
        header_id (int): Index of the header's protocol type in the generated
            type list.
        low_priority (bool, optional): Whether the server sends its transfers
            in the low-priority lane, as for :class:`ImageMulti`. Defaults to
            False.
    """

    def __init__(self, header_id: int, low_priority: bool = False) -> None:
        super().__init__(low_priority)
        self._header_id = header_id

    def get(self, index: int) -> SingleImageWithHeader[H]:
        """Return a handle for one image index.

        Args:
            index (int): Non-negative image index.

        Returns:
            SingleImageWithHeader[H]: Handle that can create, or read
                ``index``.

        Raises:
            TypeError: If ``index`` is not an integer.
        """
        return SingleImageWithHeader(self._server, self._value_id, _index(index))

    def __getitem__(self, index: int) -> SingleImageWithHeader[H]:
        return self.get(index)

    def values(self) -> Sequence[SingleImageWithHeader[H]]:
        """Return the handles of the populated indices.

        Returns:
            Sequence[SingleImageWithHeader[H]]: The handles, in ascending order
                of their indices.
        """
        return [self.get(index) for index in self.indices()]

    def items(self) -> Sequence[tuple[int, SingleImageWithHeader[H]]]:
        """Return the populated indices with their handles.

        Returns:
            Sequence[tuple[int, SingleImageWithHeader[H]]]: Pairs of an index and
                its handle, in ascending order of the indices.
        """
        return [(index, self.get(index)) for index in self.indices()]


class Map[K, V](_StaticBase):
    """Server-controlled dictionary mirrored read-only to the client.

    It supports the dict operations ``map[key]``, ``map[key] = value``,
    ``del map[key]``, ``key in map``, ``len(map)`` and iteration over its
    keys. A missing key raises :class:`KeyError`. Iteration walks a copy taken
    when it starts.
    """

    def __init__(self, key_id: int, value_id: int) -> None:
        self._key_id = key_id
        self._value_type_id = value_id

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_map(name, types[self._key_id], types[self._value_type_id])
        del self._key_id
        del self._value_type_id

    def set(self, value: dict[K, V]) -> None:
        """Set the dict in the UI dict.

        Args:
            value (dict[K, V]): The dictionary to set.
        """
        self._server.map_set(self._value_id, value)

    def get(self) -> dict[K, V]:
        """Get the dict in the UI dict.

        Returns:
            dict[K, V]: The dictionary in the UI state.
        """
        return self._server.map_get(self._value_id)

    def set_item(self, key: K, value: V) -> None:
        """Set the item in the UI dict.

        Args:
            key (K): The key of the item.
            value (V): The value of the item.
        """
        self._server.map_set_item(self._value_id, key, value)

    def get_item(self, key: K) -> V:
        """Get the item in the UI dict.

        Args:
            key (K): The key of the item.

        Returns:
            V: The value of the item.

        Raises:
            KeyError: If the dict has no such key.
        """
        return self._server.map_get_item(self._value_id, key)

    def remove_item(self, key: K) -> V:
        """Remove the item from the UI dict and return it.

        Args:
            key (K): The key of the item.

        Returns:
            V: The removed value.

        Raises:
            KeyError: If the dict has no such key.
        """
        return self._server.map_del_item(self._value_id, key)

    def __getitem__(self, key: K) -> V:
        """Get the item in the UI dict."""
        return self.get_item(key)

    def __setitem__(self, key: K, value: V) -> None:
        """Set the item in the UI dict."""
        self.set_item(key, value)

    def __delitem__(self, key: K) -> None:
        """Remove the item from the UI dict."""
        self.remove_item(key)

    def __contains__(self, key: object) -> bool:
        """Return whether the UI dict has ``key``."""
        return self._server.map_contains(self._value_id, key)

    def __len__(self) -> int:
        """Return the number of items in the UI dict."""
        return self._server.map_len(self._value_id)

    def __iter__(self) -> Iterator[K]:
        """Iterate over a copy of the keys of the UI dict."""
        return iter(self.get())


class Vec[T](_StaticBase):
    """Server-controlled list mirrored read-only to the client.

    It supports the list operations ``vec[i]``, ``vec[i] = value``,
    ``del vec[i]``, ``value in vec``, ``len(vec)`` and iteration. A negative
    index counts from the end, and an index out of range raises
    :class:`IndexError`. Iteration walks a copy taken when it starts.
    """

    def __init__(self, obj_id: int) -> None:
        self._obj_id = obj_id

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_vec(name, types[self._obj_id])
        del self._obj_id

    def set(self, value: list[T]) -> None:
        """Set the list in the UI list.

        Args:
            value (list[T]): The list to set.
        """
        self._server.list_set(self._value_id, value)

    def get(self) -> list[T]:
        """Get the list in the UI list.

        Returns:
            list[T]: The list in the UI state.
        """
        return self._server.list_get(self._value_id)

    def set_item(self, idx: int, value: T) -> None:
        """Set the item in the UI list.

        Args:
            idx (int): The index of the item; a negative index counts from
                the end.
            value (T): The value of the item.

        Raises:
            IndexError: If the index is out of range.
        """
        self._server.list_set_item(self._value_id, idx, value)

    def get_item(self, idx: int) -> T:
        """Get the item in the UI list.

        Args:
            idx (int): The index of the item; a negative index counts from
                the end.

        Returns:
            T: The value of the item.

        Raises:
            IndexError: If the index is out of range.
        """
        return self._server.list_get_item(self._value_id, idx)

    def remove_item(self, idx: int) -> T:
        """Remove the item from the UI list and return it.

        Args:
            idx (int): The index of the item; a negative index counts from
                the end.

        Returns:
            T: The removed item.

        Raises:
            IndexError: If the index is out of range.
        """
        return self._server.list_del_item(self._value_id, idx)

    def insert_item(self, idx: int, value: T) -> None:
        """Insert the item into the UI list before the item at ``idx``.

        Only the new item is sent to the client.

        Args:
            idx (int): The position of the new item, up to the length to
                append; a negative index counts from the end, as for
                ``list.insert``.
            value (T): The value of the item.

        Raises:
            IndexError: If the index is out of range. Unlike
                ``list.insert``, an index past either end is not clamped.
        """
        self._server.list_insert_item(self._value_id, idx, value)

    def add_item(self, value: T) -> None:
        """Add the item to the UI list.

        Args:
            value (T): The value of the item.
        """
        self._server.list_append_item(self._value_id, value)

    def __getitem__(self, idx: int) -> T:
        """Get the item in the UI list."""
        return self.get_item(idx)

    def __setitem__(self, idx: int, value: T) -> None:
        """Set the item in the UI list."""
        self.set_item(idx, value)

    def __delitem__(self, idx: int) -> None:
        """Remove the item from the UI list."""
        self.remove_item(idx)

    def __contains__(self, value: object) -> bool:
        """Return whether the UI list holds ``value``."""
        return value in self.get()

    def __len__(self) -> int:
        """Return the number of items in the UI list."""
        return self._server.list_len(self._value_id)

    def __iter__(self) -> Iterator[T]:
        """Iterate over a copy of the items of the UI list."""
        return iter(self.get())


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

    The data starts "not set": :meth:`get` returns ``None``, and the client
    shows no data, until :meth:`set` creates it, so an empty buffer differs
    from no data. Every other change needs data that is set and raises
    ``ValueError`` otherwise; :meth:`reset` returns the data to "not set".

    The client acknowledges :meth:`set`, :meth:`add`, :meth:`replace` and
    :meth:`remove`, and one of these changes is sent at a time: a call blocks
    until the client has acknowledged the previous one, then queues its change
    and returns. With ``wait``, the call returns only once the client has
    acknowledged its own change, or the connection has ended; without a
    client it does not wait. :meth:`clear` and :meth:`reset` are not
    acknowledged and do not wait for the client.

    Methods that take a buffer read it until they return, while other Python
    threads keep running. Do not modify the buffer from another thread during
    the call, or the client may receive a mix of old and new contents; pass a
    copy if another thread may write to it.

    Args:
        dtype (type[T]): One of ``numpy.uint8`` through ``numpy.uint64``,
            ``numpy.int8`` through ``numpy.int64``, ``numpy.float32``, or
            ``numpy.float64``.
        low_priority (bool, optional): Whether the server sends its transfers
            in the low-priority lane: only while no other state has a message
            waiting, and a window of their data at a time, so small states do
            not wait behind them. A change of another state made after a
            change of this one can then reach the client first. Defaults to
            False.
    """

    _header_id: int | None = None

    def __init__(self, dtype: type[T], low_priority: bool = False) -> None:
        self._dtype = dtype
        self._low_priority = low_priority

    def _header_type(self, types: list[PyObjectType]) -> PyObjectType | None:
        return None if self._header_id is None else types[self._header_id]

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        self._value_id = self._server.add_data(
            name, _get_dtype_id(self._dtype), self._header_type(types), self._low_priority
        )

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

    def is_idle(self) -> bool:
        """Return whether no change waits for the client's acknowledgement.

        The next acknowledged change is then sent without waiting. It is also
        true while no client is connected, because a change is then only
        stored. A producer that wants the freshest data on a slow link can
        skip a change while this is false.

        Returns:
            bool: Whether the data is idle.
        """
        return self._server.data_is_idle(self._value_id)

    def set(self, data: Buffer, wait: bool = False) -> None:
        """Set the data in the UI data, creating it when it is not set.

        Args:
            data (Buffer): Data to set. NumPy arrays and other buffer-protocol
                objects are accepted.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this change, or the connection has ended.
                Defaults to False.
        """
        self._server.data_set(self._value_id, data, None, wait)

    def add(self, data: Buffer, wait: bool = False) -> None:
        """Add the data to the UI data.

        Args:
            data (Buffer): Data to append. NumPy arrays and other
                buffer-protocol objects are accepted.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this change, or the connection has ended.
                Defaults to False.
        """
        self._server.data_add(self._value_id, data, None, wait)

    def replace(self, data: Buffer, index: int, wait: bool = False) -> None:
        """Replace the data in the UI data.

        Args:
            data (Buffer): Replacement data. NumPy arrays and other
                buffer-protocol objects are accepted.
            index (int): Index at which replacement starts.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this change, or the connection has ended.
                Defaults to False.
        """
        self._server.data_replace(self._value_id, data, index, None, wait)

    def remove(self, index: int, count: int, wait: bool = False) -> None:
        """Remove the data from the UI data.

        Args:
            index (int): Index at which removal starts.
            count (int): Number of items to remove.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this change, or the connection has ended.
                Defaults to False.
        """
        self._server.data_remove(self._value_id, index, count, None, wait)

    def clear(self) -> None:
        """Clear the data in the UI data; the data stays set, but empty."""
        self._server.data_clear(self._value_id, None)

    def reset(self) -> None:
        """Return the data, and its header, to "not set".

        Data that is not set stays so.
        """
        self._server.data_reset(self._value_id)


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
        low_priority (bool, optional): Whether the server sends its transfers
            in the low-priority lane: only while no other state has a message
            waiting, and a window of their data at a time, so small states do
            not wait behind them. A change of another state made after a
            change of this one can then reach the client first. Defaults to
            False.
    """

    def __init__(self, dtype: type[T], header_id: int, low_priority: bool = False) -> None:
        super().__init__(dtype, low_priority)
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

    def set(self, data: Buffer, wait: bool = False, header: H | _Unset = UNSET) -> None:
        """Set the data and optionally the header.

        Args:
            data (Buffer): Data to set.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this change, or the connection has ended.
                Defaults to False.
            header (H | _Unset, optional): New header; :data:`UNSET` keeps the
                current one. Defaults to :data:`UNSET`.

        Raises:
            ValueError: If the data is not set and no header is given.
        """
        self._server.data_set(self._value_id, data, _header(header), wait)

    def add(self, data: Buffer, wait: bool = False, header: H | _Unset = UNSET) -> None:
        """Append data and optionally replace the header.

        Args:
            data (Buffer): Data to append.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this change, or the connection has ended.
                Defaults to False.
            header (H | _Unset, optional): New header; :data:`UNSET` keeps the
                current one. Defaults to :data:`UNSET`.
        """
        self._server.data_add(self._value_id, data, _header(header), wait)

    def replace(self, data: Buffer, index: int, wait: bool = False, header: H | _Unset = UNSET) -> None:
        """Replace data from ``index`` and optionally the header.

        Args:
            data (Buffer): Replacement data.
            index (int): Index at which replacement starts.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this change, or the connection has ended.
                Defaults to False.
            header (H | _Unset, optional): New header; :data:`UNSET` keeps the
                current one. Defaults to :data:`UNSET`.
        """
        self._server.data_replace(self._value_id, data, index, _header(header), wait)

    def remove(self, index: int, count: int, wait: bool = False, header: H | _Unset = UNSET) -> None:
        """Remove data and optionally replace the header.

        Args:
            index (int): Index at which removal starts.
            count (int): Number of items to remove.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this change, or the connection has ended.
                Defaults to False.
            header (H | _Unset, optional): New header; :data:`UNSET` keeps the
                current one. Defaults to :data:`UNSET`.
        """
        self._server.data_remove(self._value_id, index, count, _header(header), wait)

    def clear(self, header: H | _Unset = UNSET) -> None:
        """Clear the data; the header stays unless a new one is given.

        Args:
            header (H | _Unset, optional): New header; :data:`UNSET` keeps the
                current one. Defaults to :data:`UNSET`.
        """
        self._server.data_clear(self._value_id, _header(header))

    def set_header(self, header: H) -> None:
        """Replace the header alone.

        The client does not acknowledge a header, so this does not wait for
        the client itself: headers set faster than the client reads them
        queue up, as values do. It is queued after a buffer change that
        another thread is still sending, though, and waits as long as that
        change waits for the client.

        Args:
            header (H): New header.

        Raises:
            ValueError: If the data is not set.
        """
        self._server.data_set_header(self._value_id, header)


class DataTake[T: np.generic](_StaticBase):
    """One-shot numeric buffer sent to and consumed by the client.

    Methods that take a buffer read it until they return, while other Python
    threads keep running. Do not modify the buffer from another thread during
    the call, or the client may receive a mix of old and new contents; pass a
    copy if another thread may write to it.

    Args:
        dtype (type[T]): One of the integer or floating-point NumPy scalar
            types supported by :class:`Data`.
        low_priority (bool, optional): Whether the server sends its transfers
            in the low-priority lane: only while no other state has a message
            waiting, and a window of their data at a time, so small states do
            not wait behind them. A change of another state made after a
            change of this one can then reach the client first. Defaults to
            False.
    """

    _header_id: int | None = None

    def __init__(self, dtype: type[T], low_priority: bool = False) -> None:
        self._dtype = dtype
        self._low_priority = low_priority

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        header = None if self._header_id is None else types[self._header_id]
        self._value_id = self._server.add_data_take(name, _get_dtype_id(self._dtype), header, self._low_priority)

    def set(
        self,
        data: Buffer,
        blocking: bool = False,
        cache: bool = False,
        wait: bool = False,
    ) -> None:
        """Set the data in the UI DataTake.

        DataTake does not have a get method because the data is not stored in
        the server. It is an alternative to Signal with the opposite transport
        direction.

        Transfers are sent one at a time: the call first waits until the
        client has received the previous transfer, or taken it if it
        was blocking. With no client connected it does not wait, and the data
        is dropped unless ``cache`` keeps it for the next client.

        Args:
            data (Buffer): Data to send. NumPy arrays and other buffer-protocol
                objects are accepted.
            blocking (bool, optional): Whether the client acknowledges this
                data only once the UI takes it, so the next send waits until
                then. Defaults to False.
            cache (bool, optional): Whether to cache the last sent data so a
                newly initialized client receives it. Defaults to False.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this data, by receiving it or, with ``blocking``,
                taking it, or the connection has ended. Defaults to False.
        """
        self._server.data_take_set(self._value_id, data, blocking, cache, None, wait)

    def is_idle(self) -> bool:
        """Return whether no transfer waits for the client.

        The next :meth:`set` is then sent at once. It is also true while no
        client is connected.

        Returns:
            bool: Whether the DataTake is idle.
        """
        return self._server.data_take_is_idle(self._value_id)


class DataTakeWithHeader[T: np.generic, H](DataTake[T]):
    """:class:`DataTake` whose transfers carry a typed header ``H``.

    Every transfer carries a header.

    Args:
        dtype (type[T]): One of the NumPy scalar types supported by
            :class:`Data`.
        header_id (int): Index of the header's protocol type in the generated
            type list.
        low_priority (bool, optional): Whether the server sends its transfers
            in the low-priority lane: only while no other state has a message
            waiting, and a window of their data at a time, so small states do
            not wait behind them. A change of another state made after a
            change of this one can then reach the client first. Defaults to
            False.
    """

    def __init__(self, dtype: type[T], header_id: int, low_priority: bool = False) -> None:
        super().__init__(dtype, low_priority)
        self._header_id = header_id

    def set(
        self,
        data: Buffer,
        blocking: bool = False,
        cache: bool = False,
        wait: bool = False,
        header: H | _Unset = UNSET,
    ) -> None:
        """Send data with its header, as :meth:`DataTake.set`.

        Args:
            data (Buffer): Data to send.
            blocking (bool, optional): Whether the client acknowledges this
                data only once the UI takes it, so the next send waits until
                then. Defaults to False.
            cache (bool, optional): Whether to cache the last sent data and
                header for a newly initialized client. Defaults to False.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this data, by receiving it or, with ``blocking``,
                taking it, or the connection has ended. Defaults to False.
            header (H | _Unset, optional): Header sent with the data; it is
                required, :data:`UNSET` raises. Defaults to :data:`UNSET`.

        Raises:
            ValueError: If no header is given.
        """
        self._server.data_take_set(self._value_id, data, blocking, cache, _header(header), wait)


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

    def is_idle(self) -> bool:
        """Return whether no change of this index waits for the client.

        See :meth:`DataMulti.is_idle`.

        Returns:
            bool: Whether this index is idle.
        """
        return self._server.data_multi_is_idle(self._value_id, self._index)

    def get(self) -> npt.NDArray[T]:
        """Return a locally mutable snapshot of the buffer at this index.

        Mutating the array does not update the server state or client; call
        :meth:`set` to publish modified data.

        Returns:
            npt.NDArray[T]: A NumPy view backed by a copied ``bytearray``.
        """
        data = self._server.data_multi_get(self._value_id, self._index)
        return np.frombuffer(data, dtype=self._dtype)

    def set(self, data: Buffer, wait: bool = False) -> None:
        """Set the data in the UI data at this index.

        Args:
            data (Buffer): Data to set. NumPy arrays and other buffer-protocol
                objects are accepted.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this change, or the connection has ended.
                Defaults to False.
        """
        self._server.data_multi_set(self._value_id, self._index, data, None, wait)

    def add(self, data: Buffer, wait: bool = False) -> None:
        """Add the data to the UI data at this index.

        Args:
            data (Buffer): Data to append. NumPy arrays and other
                buffer-protocol objects are accepted.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this change, or the connection has ended.
                Defaults to False.
        """
        self._server.data_multi_add(self._value_id, self._index, data, None, wait)

    def replace(self, data: Buffer, index: int, wait: bool = False) -> None:
        """Replace the data in the UI data at this index.

        Args:
            data (Buffer): Replacement data. NumPy arrays and other
                buffer-protocol objects are accepted.
            index (int): Index at which replacement starts within this buffer.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this change, or the connection has ended.
                Defaults to False.
        """
        self._server.data_multi_replace(self._value_id, self._index, data, index, None, wait)

    def remove(self, index: int, count: int, wait: bool = False) -> None:
        """Remove the data from the UI data at this index.

        Args:
            index (int): Index at which removal starts within this buffer.
            count (int): Number of items to remove.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this change, or the connection has ended.
                Defaults to False.
        """
        self._server.data_multi_remove(self._value_id, self._index, index, count, None, wait)

    def clear(self) -> None:
        """Clear the data in the UI data at this index."""
        self._server.data_multi_clear(self._value_id, self._index, None)


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

    def set(self, data: Buffer, wait: bool = False, header: H | _Unset = UNSET) -> None:
        """Set the data at this index and optionally its header.

        Args:
            data (Buffer): Data to set.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this change, or the connection has ended.
                Defaults to False.
            header (H | _Unset, optional): New header; :data:`UNSET` keeps the
                current one. Defaults to :data:`UNSET`.

        Raises:
            ValueError: If the index does not exist and no header is given.
        """
        self._server.data_multi_set(self._value_id, self._index, data, _header(header), wait)

    def add(self, data: Buffer, wait: bool = False, header: H | _Unset = UNSET) -> None:
        """Append data at this index and optionally replace its header.

        Args:
            data (Buffer): Data to append.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this change, or the connection has ended.
                Defaults to False.
            header (H | _Unset, optional): New header; :data:`UNSET` keeps the
                current one. Defaults to :data:`UNSET`.
        """
        self._server.data_multi_add(self._value_id, self._index, data, _header(header), wait)

    def replace(self, data: Buffer, index: int, wait: bool = False, header: H | _Unset = UNSET) -> None:
        """Replace data from ``index`` and optionally the header.

        Args:
            data (Buffer): Replacement data.
            index (int): Index at which replacement starts within this buffer.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this change, or the connection has ended.
                Defaults to False.
            header (H | _Unset, optional): New header; :data:`UNSET` keeps the
                current one. Defaults to :data:`UNSET`.
        """
        self._server.data_multi_replace(self._value_id, self._index, data, index, _header(header), wait)

    def remove(self, index: int, count: int, wait: bool = False, header: H | _Unset = UNSET) -> None:
        """Remove data at this index and optionally replace its header.

        Args:
            index (int): Index at which removal starts within this buffer.
            count (int): Number of items to remove.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this change, or the connection has ended.
                Defaults to False.
            header (H | _Unset, optional): New header; :data:`UNSET` keeps the
                current one. Defaults to :data:`UNSET`.
        """
        self._server.data_multi_remove(self._value_id, self._index, index, count, _header(header), wait)

    def clear(self, header: H | _Unset = UNSET) -> None:
        """Clear the data at this index; its header stays unless a new one is given.

        Args:
            header (H | _Unset, optional): New header; :data:`UNSET` keeps the
                current one. Defaults to :data:`UNSET`.
        """
        self._server.data_multi_clear(self._value_id, self._index, _header(header))

    def set_header(self, header: H) -> None:
        """Replace the header at this index alone.

        It does not wait for the client itself, but it is queued after a
        buffer change to this index that another thread is still sending.

        Args:
            header (H): New header.

        Raises:
            ValueError: If the index does not exist.
        """
        self._server.data_multi_set_header(self._value_id, self._index, header)


class DataMulti[T: np.generic](_StaticBase):
    """Collection of numeric buffers indexed by non-negative integers.

    Changes wait for the client as in :class:`Data`, each index on its own:
    a change waits for the previous acknowledged change of its index only, so
    changes of different indices can reach the client in another order than
    they were made.

    ``data[index]`` returns the handle of an index, whether it holds data or
    not. ``index in data``, ``len(data)`` and iteration cover the indices that
    hold data only; iteration walks a copy taken when it starts.
    :meth:`values` and :meth:`items` return the handles of those indices, as
    for a dict.

    Args:
        dtype (type[T]): One of the integer or floating-point NumPy scalar
            types supported by :class:`Data`.
        low_priority (bool, optional): Whether the server sends its transfers
            in the low-priority lane: only while no other state has a message
            waiting, and a window of their data at a time, so small states do
            not wait behind them. A change of another state made after a
            change of this one can then reach the client first. Defaults to
            False.
    """

    _header_id: int | None = None

    def __init__(self, dtype: type[T], low_priority: bool = False) -> None:
        self._dtype = dtype
        self._low_priority = low_priority

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        header = None if self._header_id is None else types[self._header_id]
        self._value_id = self._server.add_data_multi(name, _get_dtype_id(self._dtype), header, self._low_priority)

    def get(self, index: int) -> SingleData[T]:
        """Get the SingleData object for the given index.

        Args:
            index (int): The index of the ``SingleData`` object.

        Returns:
            SingleData[T]: The ``SingleData`` object for the given index.

        Raises:
            TypeError: If ``index`` is not an integer.
        """
        return SingleData(self._dtype, self._server, self._value_id, _index(index))

    def is_idle(self, index: int | None = None) -> bool:
        """Return whether no change waits for the client.

        See :meth:`Data.is_idle`.

        Args:
            index (int | None, optional): The index to ask about. Defaults to
                None, which asks about every index.

        Returns:
            bool: Whether the index, or every index, is idle.
        """
        return self._server.data_multi_is_idle(self._value_id, index)

    def remove_index(self, index: int) -> None:
        """Remove the given index and its header from the DataMulti.

        Args:
            index (int): The index to remove.
        """
        self._server.data_multi_remove_index(self._value_id, index)

    def reset(self) -> None:
        """Reset (clear all indices and headers) in the DataMulti."""
        self._server.data_multi_reset(self._value_id)

    def indices(self) -> list[int]:
        """Return the indices that hold data.

        Returns:
            list[int]: The indices in ascending order.
        """
        return self._server.data_multi_indices(self._value_id)

    def __len__(self) -> int:
        return self._server.data_multi_len(self._value_id)

    def __contains__(self, index: object) -> bool:
        try:
            index = _index(index)
        except TypeError:
            return False
        return 0 <= index <= _MAX_INDEX and self._server.data_multi_contains(self._value_id, index)

    def __getitem__(self, index: int) -> SingleData[T]:
        return self.get(index)

    def __iter__(self) -> Iterator[int]:
        """Iterate over a copy of the indices that hold data."""
        return iter(self.indices())

    def values(self) -> Sequence[SingleData[T]]:
        """Return the handles of the indices that hold data.

        Returns:
            Sequence[SingleData[T]]: The handles, in ascending order of their
                indices.
        """
        return [self.get(index) for index in self.indices()]

    def items(self) -> Sequence[tuple[int, SingleData[T]]]:
        """Return the indices that hold data with their handles.

        Returns:
            Sequence[tuple[int, SingleData[T]]]: Pairs of an index and its
                handle, in ascending order of the indices.
        """
        return [(index, self.get(index)) for index in self.indices()]


class DataMultiWithHeader[T: np.generic, H](DataMulti[T]):
    """:class:`DataMulti` whose buffers each carry a typed header ``H``.

    Args:
        dtype (type[T]): One of the NumPy scalar types supported by
            :class:`Data`.
        header_id (int): Index of the header's protocol type in the generated
            type list.
        low_priority (bool, optional): Whether the server sends its transfers
            in the low-priority lane: only while no other state has a message
            waiting, and a window of their data at a time, so small states do
            not wait behind them. A change of another state made after a
            change of this one can then reach the client first. Defaults to
            False.
    """

    def __init__(self, dtype: type[T], header_id: int, low_priority: bool = False) -> None:
        super().__init__(dtype, low_priority)
        self._header_id = header_id

    def get(self, index: int) -> SingleDataWithHeader[T, H]:
        """Get the SingleDataWithHeader object for the given index.

        Args:
            index (int): The index of the ``SingleDataWithHeader`` object.

        Returns:
            SingleDataWithHeader[T, H]: The handle for the given index.

        Raises:
            TypeError: If ``index`` is not an integer.
        """
        return SingleDataWithHeader(self._dtype, self._server, self._value_id, _index(index))

    def __getitem__(self, index: int) -> SingleDataWithHeader[T, H]:
        return self.get(index)

    def values(self) -> Sequence[SingleDataWithHeader[T, H]]:
        """Return the handles of the indices that hold data.

        Returns:
            Sequence[SingleDataWithHeader[T, H]]: The handles, in ascending order
                of their indices.
        """
        return [self.get(index) for index in self.indices()]

    def items(self) -> Sequence[tuple[int, SingleDataWithHeader[T, H]]]:
        """Return the indices that hold data with their handles.

        Returns:
            Sequence[tuple[int, SingleDataWithHeader[T, H]]]: Pairs of an index
                and its handle, in ascending order of the indices.
        """
        return [(index, self.get(index)) for index in self.indices()]


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

    def set(self, data: Buffer, blocking: bool = False, cache: bool = False, wait: bool = False) -> None:
        """Set the data in the UI DataMultiTake at this index.

        Transfers are sent one at a time: the call first waits until the
        client has received the previous transfer of this index, or taken it if it
        was blocking. With no client connected it does not wait, and the data
        is dropped unless ``cache`` keeps it for the next client.

        Args:
            data (Buffer): Data to send. NumPy arrays and other buffer-protocol
                objects are accepted.
            blocking (bool, optional): Whether the client acknowledges this
                data only once the UI takes it, so the next send waits until
                then. Defaults to False.
            cache (bool, optional): Whether to cache the last sent data for a
                newly initialized client, which holds only cached indices.
                Defaults to False.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this data, by receiving it or, with ``blocking``,
                taking it, or the connection has ended. Defaults to False.
        """
        self._server.data_multi_take_set(
            self._value_id,
            self._index,
            data,
            blocking,
            cache,
            None,
            wait,
        )

    def is_idle(self) -> bool:
        """Return whether no transfer of this index waits for the client.

        Returns:
            bool: Whether this index is idle.
        """
        return self._server.data_multi_take_is_idle(self._value_id, self._index)


class SingleDataTakeWithHeader[T: np.generic, H](SingleDataTake[T]):
    """:class:`SingleDataTake` of a :class:`DataMultiTakeWithHeader`."""

    def set(
        self,
        data: Buffer,
        blocking: bool = False,
        cache: bool = False,
        wait: bool = False,
        header: H | _Unset = UNSET,
    ) -> None:
        """Send data at this index with its header, as :meth:`SingleDataTake.set`.

        Args:
            data (Buffer): Data to send.
            blocking (bool, optional): Whether the client acknowledges this
                data only once the UI takes it, so the next send waits until
                then. Defaults to False.
            cache (bool, optional): Whether to cache the last sent data and
                header for a newly initialized client, which holds only cached
                indices. Defaults to False.
            wait (bool, optional): Whether to return only once the client has
                acknowledged this data, by receiving it or, with ``blocking``,
                taking it, or the connection has ended. Defaults to False.
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
            cache,
            _header(header),
            wait,
        )


class DataMultiTake[T: np.generic](_StaticBase):
    """Keyed one-shot numeric buffers sent to and consumed by the client.

    ``data[index]`` returns the handle of an index. The server keeps no list
    of the indices, so the state is not iterable and does not support ``in``;
    both raise :class:`TypeError`.

    Args:
        dtype (type[T]): One of the integer or floating-point NumPy scalar
            types supported by :class:`Data`.
        low_priority (bool, optional): Whether the server sends its transfers
            in the low-priority lane: only while no other state has a message
            waiting, and a window of their data at a time, so small states do
            not wait behind them. A change of another state made after a
            change of this one can then reach the client first. Defaults to
            False.
    """

    _header_id: int | None = None

    def __init__(self, dtype: type[T], low_priority: bool = False) -> None:
        self._dtype = dtype
        self._low_priority = low_priority

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        header = None if self._header_id is None else types[self._header_id]
        self._value_id = self._server.add_data_multi_take(
            name,
            _get_dtype_id(self._dtype),
            header,
            self._low_priority,
        )

    def get(self, index: int) -> SingleDataTake[T]:
        """Get the SingleDataTake object for the given index.

        Args:
            index (int): The index of the ``SingleDataTake`` object.

        Returns:
            SingleDataTake[T]: The ``SingleDataTake`` object for the given
                index.

        Raises:
            TypeError: If ``index`` is not an integer.
        """
        return SingleDataTake(self._dtype, self._server, self._value_id, _index(index))

    def remove_index(self, index: int) -> None:
        """Remove the given index from the DataMultiTake.

        Args:
            index (int): The index to remove.
        """
        self._server.data_multi_take_remove_index(self._value_id, index)

    def is_idle(self, index: int | None = None) -> bool:
        """Return whether no transfer waits for the client.

        See :meth:`DataTake.is_idle`; each index has its own transfer.

        Args:
            index (int | None, optional): The index to ask about. Defaults to
                None, which asks about every index.

        Returns:
            bool: Whether the index, or every index, is idle.
        """
        return self._server.data_multi_take_is_idle(self._value_id, index)

    def reset(self) -> None:
        """Reset (clear all indices) in the DataMultiTake."""
        self._server.data_multi_take_reset(self._value_id)

    def __getitem__(self, index: int) -> SingleDataTake[T]:
        return self.get(index)

    # The server keeps no list of the indices, so there is nothing to iterate
    # over. Without this, ``iter`` and ``in`` would fall back to
    # ``__getitem__``, which accepts every index, and never end.
    __iter__: None = None


class DataMultiTakeWithHeader[T: np.generic, H](DataMultiTake[T]):
    """:class:`DataMultiTake` whose transfers carry a typed header ``H``.

    Args:
        dtype (type[T]): One of the NumPy scalar types supported by
            :class:`Data`.
        header_id (int): Index of the header's protocol type in the generated
            type list.
        low_priority (bool, optional): Whether the server sends its transfers
            in the low-priority lane: only while no other state has a message
            waiting, and a window of their data at a time, so small states do
            not wait behind them. A change of another state made after a
            change of this one can then reach the client first. Defaults to
            False.
    """

    def __init__(self, dtype: type[T], header_id: int, low_priority: bool = False) -> None:
        super().__init__(dtype, low_priority)
        self._header_id = header_id

    def get(self, index: int) -> SingleDataTakeWithHeader[T, H]:
        """Get the SingleDataTakeWithHeader object for the given index.

        Args:
            index (int): The index of the ``SingleDataTakeWithHeader`` object.

        Returns:
            SingleDataTakeWithHeader[T, H]: The handle for the given index.

        Raises:
            TypeError: If ``index`` is not an integer.
        """
        return SingleDataTakeWithHeader(self._dtype, self._server, self._value_id, _index(index))

    def __getitem__(self, index: int) -> SingleDataTakeWithHeader[T, H]:
        return self.get(index)


_TRANSFER_OFFER = 0
_TRANSFER_FINISHED = 1


class _TransferEndpoint(_CallbackBase):
    """The callbacks every transfer endpoint has."""

    _meta_id: int | None = None
    _download: bool

    def __init__(self) -> None:
        self._finished: list[Callable[[TransferHandle], Any]] = []
        self._dispatching = False

    def _dispatch_event(self, kind: int) -> None:
        if kind == _TRANSFER_FINISHED:
            # The transfer travels with the event this worker is delivering.
            handle = self._server.transfer_finished(self._value_id, self._download)
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

    def connect[F: Callable[[TransferHandle], Any]](self, callback: F) -> F:
        """Call ``callback`` once for every transfer when it is done or failed.

        The callback runs on a callback worker; exceptions go to the server's
        error handler.

        Args:
            callback (Callable[[TransferHandle], Any]): Receives the finished
                transfer.

        Returns:
            Callable[[TransferHandle], Any]: ``callback``, so the method also works as
                a decorator.
        """
        self._finished.append(callback)
        self._update_dispatch()
        return callback

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


class _DownloadBase(_TransferEndpoint):
    """What :class:`Download` and :class:`DownloadWithMeta` share."""

    _download = True

    def _initialize(self, name: str, types: list[PyObjectType]) -> None:
        meta = None if self._meta_id is None else types[self._meta_id]
        self._value_id = self._server.add_download(name, meta)

    @staticmethod
    def _meta(meta: object) -> tuple[object] | None:
        return None

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


class Download(_DownloadBase):
    """Endpoint that sends files from the server to the client.

    The client receives each offer and decides where the data goes; the data
    is read once the client accepts. The endpoint carries at most one
    unfinished transfer at a time: a send while it is busy raises, or waits
    with ``wait=True``. Do not wait inside a callback: it holds a callback
    worker for as long as it waits.
    """

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
            name (str): The name of the set. It follows the rules of a file
                name: relative, with ``/`` between folders, without ``..`` or
                a backslash.
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
                two files have the same name, ``name`` breaks the rules, or
                ``timeout`` is given without ``wait``.
        """
        return self._send_files(paths, name, root, None, wait, timeout)

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
                or a NumPy array. It must be C-contiguous: pass
                ``np.ascontiguousarray(a)`` for a reversed or stepped view.
                It is copied once.
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
            ValueError: If the data is not C-contiguous, the name or metadata
                is invalid, or ``timeout`` is given without ``wait``.
        """
        return self._send_bytes(data, name, None, wait, timeout)


class DownloadWithMeta[M](_DownloadBase):
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

    def send_file(
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

    def send_files(
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

    def send_bytes(
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
            data (Buffer): The C-contiguous data; it is copied once.
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

    The receiver set with :meth:`on_offer` decides where each offer goes;
    without one, offers are rejected with "no receiver". The endpoint carries
    at most one unfinished transfer at a time.
    """

    _download = False

    def __init__(self) -> None:
        super().__init__()
        self._receiver: Callable[[TransferOffer], Any] | None = None

    def _wants_events(self) -> bool:
        return self._receiver is not None or bool(self._finished)

    def _dispatch_offer(self) -> None:
        receiver = self._receiver
        offer = self._server.transfer_take_offer(self._value_id)
        if offer is None:
            return
        try:
            if receiver is not None:
                self._signals_manager._invoke(receiver, TransferOffer(offer))
        finally:
            offer.close("the offer was not answered")

    def on_offer[F: Callable[[TransferOffer], Any] | None](self, func: F) -> F:
        """Set the function that answers the client's offers.

        It runs on a callback worker and must accept or reject the offer
        before it returns; an offer it leaves unanswered, or one it raises
        for, is rejected. Exceptions go to the server's error handler.

        Args:
            func (Callable[[TransferOffer], Any] | None): The receiver, or
                ``None`` to reject offers again.

        Returns:
            Callable[[TransferOffer], Any] | None: ``func``, so the method
                also works as a decorator.
        """
        had_receiver = self._receiver is not None
        self._receiver = func
        self._update_dispatch()
        if func is not None and not had_receiver:
            self._server.transfer_set_receiver(self._value_id, True)
        elif func is None and had_receiver:
            self._server.transfer_set_receiver(self._value_id, False)
        return func

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
    "denu",
    "vunit",
    "vnamed",
    "vtuple",
    "cl",
    "st",
    "vec",
    "opt",
    "li",
    "tu",
    "map",
    "ImageColor",
    "_CustomStruct",
    "_CustomEnum",
    "_TupleVariant",
    "PyObjectType",
    "UNSET",
]
