"""Low-level native extension used by generated Python server bindings.

Applications normally use :mod:`egui_states.structures` and a generated
``StatesServer`` instead of constructing these objects directly.
"""

from collections.abc import Buffer
from enum import IntEnum
from os import PathLike
from pathlib import Path
from typing import Any

from egui_states.structures import _CustomStruct

class PyObjectType:
    """Opaque protocol type descriptor consumed by generated bindings."""

# Primitive protocol type descriptors.
u8: PyObjectType
u16: PyObjectType
u32: PyObjectType
u64: PyObjectType
i8: PyObjectType
i16: PyObjectType
i32: PyObjectType
i64: PyObjectType
f32: PyObjectType
f64: PyObjectType
bo: PyObjectType
st: PyObjectType
emp: PyObjectType

def opt(pytype: PyObjectType) -> PyObjectType: ...
def tu(elements: list[PyObjectType]) -> PyObjectType: ...
def cl(elements: list[PyObjectType], class_type: type[_CustomStruct]) -> PyObjectType: ...
def li(element_type: PyObjectType, size: int) -> PyObjectType: ...
def vec(element_type: PyObjectType) -> PyObjectType: ...
def map(key_type: PyObjectType, value_type: PyObjectType) -> PyObjectType: ...
def enu(enum_obj: type[IntEnum]) -> PyObjectType: ...

class StateServerCore:
    """Low-level native WebSocket server and synchronized-state registry."""

    def __init__(
        self,
        version: int | None = None,
    ) -> None: ...
    def start(
        self,
        port: int,
        ip_addr: tuple[int, int, int, int] | None = None,
        token: str | None = None,
    ) -> None: ...
    def stop(self) -> None: ...
    def is_running(self) -> bool: ...
    def is_connected(self) -> bool: ...
    def disconnect_client(self) -> None: ...
    def serve_web(self, path: str | PathLike[str]) -> None: ...
    def stop_web(self) -> None: ...
    def web_path(self) -> Path | None: ...
    def update(self, duration: float | None = None) -> None: ...
    def id_to_name(self, value_id: int) -> str:
        """Return the registered state path for a protocol id.

        Args:
            value_id (int): Protocol id of the state.

        Returns:
            str: Fully qualified state path.
        """

    # values ----------------------------------------------------------------------
    def value_set(self, value_id: int, value: object, set_signal: bool, update: bool) -> None: ...
    def value_get(self, value_id: int) -> Any: ...

    # values take -----------------------------------------------------------------
    def value_take_set(self, value_id: int, value: object, blocking: bool, update: bool) -> None: ...

    # static ----------------------------------------------------------------------
    def static_set(self, value_id: int, value: object, update: bool) -> None: ...
    def static_get(self, value_id: int) -> Any: ...

    # signals ---------------------------------------------------------------------
    def signal_set(self, value_id: int, value: object) -> None: ...
    def signal_register(self, value_id: int, register: bool, with_previous: bool) -> None:
        """Configure callbacks and previous-value tracking for a state.

        Args:
            value_id (int): Protocol id of the state.
            register (bool): Whether the state should emit callback events.
            with_previous (bool): Whether events should retain the previous
                value.
        """
    def signal_get(self, last_id: int | None) -> tuple[int, Any, bool, Any] | None:
        """Wait for and claim the next callback event.

        Args:
            last_id (int | None): Id of the event claimed by the preceding call,
                which this call releases, or ``None`` on the first call.

        Returns:
            tuple[int, Any, bool, Any] | None: ``(value_id, value, has_previous,
                previous)`` for the claimed event, or ``None`` after
                ``signals_close``.
        """
    def signals_close(self) -> None:
        """Make every current and future ``signal_get`` return ``None``."""
    def signal_set_to_queue(self, value_id: int) -> None: ...
    def signal_set_to_single(self, value_id: int) -> None: ...

    # lists -----------------------------------------------------------------------
    def list_set(self, value_id: int, value: list[Any], update: bool) -> None: ...
    def list_get(self, value_id: int) -> list[Any]: ...
    def list_set_item(self, value_id: int, idx: int, value: object, update: bool) -> None: ...
    def list_get_item(self, value_id: int, idx: int) -> Any: ...
    def list_del_item(self, value_id: int, idx: int, update: bool) -> Any: ...
    def list_append_item(self, value_id: int, value: object, update: bool) -> None: ...
    def list_len(self, value_id: int) -> int: ...

    # map ------------------------------------------------------------------------
    def map_set(self, value_id: int, value: dict[Any, Any], update: bool) -> None: ...
    def map_get(self, value_id: int) -> dict[Any, Any]: ...
    def map_set_item(self, value_id: int, key: object, value: object, update: bool) -> None: ...
    def map_get_item(self, value_id: int, key: object) -> Any: ...
    def map_del_item(self, value_id: int, key: object, update: bool) -> Any: ...
    def map_len(self, value_id: int) -> int: ...

    # image -----------------------------------------------------------------------
    def image_set(
        self,
        value_id: int,
        image: Buffer,
        update: bool,
    ) -> None: ...
    def image_set_all(
        self,
        value_id: int,
        shape: list[int] | tuple[int, int],
        color: int | tuple[int, int] | tuple[int, int, int] | tuple[int, int, int, int],
        update: bool,
    ) -> None: ...
    def image_update(
        self,
        value_id: int,
        image: Buffer,
        origin: list[int] | tuple[int, int],
        update: bool,
        force: bool = False,
    ) -> None: ...
    def image_get(self, value_id: int) -> tuple[bytearray, tuple[int, int]]: ...
    def image_size(self, value_id: int) -> tuple[int, int]: ...

    # image multi -----------------------------------------------------------------
    def image_multi_set(
        self,
        value_id: int,
        index: int,
        image: Buffer,
        update: bool,
    ) -> None: ...
    def image_multi_set_all(
        self,
        value_id: int,
        index: int,
        shape: list[int] | tuple[int, int],
        color: int | tuple[int, int] | tuple[int, int, int] | tuple[int, int, int, int],
        update: bool,
    ) -> None: ...
    def image_multi_update(
        self,
        value_id: int,
        index: int,
        image: Buffer,
        origin: list[int] | tuple[int, int],
        update: bool,
        force: bool = False,
    ) -> None: ...
    def image_multi_get(self, value_id: int, index: int) -> tuple[bytearray, tuple[int, int]]: ...
    def image_multi_size(self, value_id: int, index: int) -> tuple[int, int]: ...
    def image_multi_remove_index(self, value_id: int, index: int, update: bool) -> None: ...
    def image_multi_reset(self, value_id: int, update: bool) -> None: ...
    def image_multi_len(self, value_id: int) -> int: ...
    def image_multi_contains(self, value_id: int, index: int) -> bool: ...
    def image_multi_indices(self, value_id: int) -> list[int]: ...

    # data ------------------------------------------------------------------------
    def data_get(self, value_id: int) -> bytearray: ...
    def data_set(self, value_id: int, data: Buffer, update: bool) -> None: ...
    def data_add(self, value_id: int, data: Buffer, update: bool) -> None: ...
    def data_replace(self, value_id: int, data: Buffer, index: int, update: bool) -> None: ...
    def data_remove(self, value_id: int, index: int, count: int, update: bool) -> None: ...
    def data_clear(self, value_id: int, update: bool) -> None: ...

    # data take -------------------------------------------------------------------
    def data_take_set(self, value_id: int, data: Buffer, blocking: bool, update: bool, cache: bool) -> None:
        """Send a one-shot buffer to the client.

        Args:
            value_id (int): Protocol id of the data state.
            data (Buffer): Buffer-protocol object to send.
            blocking (bool): Whether to wait until the client consumes the
                preceding value before sending.
            update (bool): Whether to request a client repaint.
            cache (bool): Whether to retain the data for a newly initialized
                client.
        """

    # data multi --------------------------------------------------------------------
    def data_multi_get(self, value_id: int, index: int) -> bytearray: ...
    def data_multi_set(self, value_id: int, index: int, data: Buffer, update: bool) -> None: ...
    def data_multi_add(self, value_id: int, index: int, data: Buffer, update: bool) -> None: ...
    def data_multi_replace(self, value_id: int, index: int, data: Buffer, data_index: int, update: bool) -> None: ...
    def data_multi_remove(self, value_id: int, index: int, data_index: int, count: int, update: bool) -> None: ...
    def data_multi_clear(self, value_id: int, index: int, update: bool) -> None: ...
    def data_multi_remove_index(self, value_id: int, index: int, update: bool) -> None: ...
    def data_multi_reset(self, value_id: int, update: bool) -> None: ...

    # data multi take ---------------------------------------------------------------
    def data_multi_take_set(
        self,
        value_id: int,
        index: int,
        data: Buffer,
        blocking: bool,
        update: bool,
        cache: bool,
    ) -> None:
        """Send one keyed, one-shot buffer to the client.

        Args:
            value_id (int): Protocol id of the data state.
            index (int): Non-negative buffer index.
            data (Buffer): Buffer-protocol object to send.
            blocking (bool): Whether to wait until the client consumes the
                preceding value before sending.
            update (bool): Whether to request a client repaint.
            cache (bool): Whether to retain the data for a newly initialized
                client.
        """
    def data_multi_take_remove_index(self, value_id: int, index: int, update: bool) -> None: ...
    def data_multi_take_reset(self, value_id: int, update: bool) -> None: ...

    # add states ------------------------------------------------------------------
    def add_value(self, name: str, object_type: PyObjectType, initial_value: object, queue: bool) -> int: ...
    def add_value_take(self, name: str, object_type: PyObjectType) -> int: ...
    def add_static(self, name: str, object_type: PyObjectType, initial_value: object) -> int: ...
    def add_signal(self, name: str, object_type: PyObjectType, queue: bool) -> int: ...
    def add_vec(self, name: str, object_type: PyObjectType) -> int: ...
    def add_map(self, name: str, key_type: PyObjectType, value_type: PyObjectType) -> int: ...
    def add_image(self, name: str) -> int: ...
    def add_image_multi(self, name: str) -> int: ...
    def add_data(self, name: str, data_type: int) -> int: ...
    def add_data_take(self, name: str, data_type: int) -> int: ...
    def add_data_multi(self, name: str, data_type: int) -> int: ...
    def add_data_multi_take(self, name: str, data_type: int) -> int: ...
    def finalize(self) -> None:
        """Freeze state registration and prepare the handshake layout."""

__all__ = [
    "u8",
    "u16",
    "u32",
    "u64",
    "i8",
    "i16",
    "i32",
    "i64",
    "f32",
    "f64",
    "bo",
    "st",
    "emp",
    "opt",
    "tu",
    "cl",
    "li",
    "vec",
    "map",
    "enu",
]
