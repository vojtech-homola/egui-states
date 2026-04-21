# ruff: noqa: D103, E402, E303, PLR0915
from __future__ import annotations

import socket
import sys
import threading
import time
from pathlib import Path

import numpy as np
import pytest

THIS_DIR = Path(__file__).resolve().parent
if str(THIS_DIR) not in sys.path:
    sys.path.insert(0, str(THIS_DIR))

from states_server import (
    StatesServer,
    TestEnum as ExampleTestEnum,
    TestEnum2 as ExampleTestEnum2,
    TestStruct as ExampleTestStruct,
    TestStruct2 as ExampleTestStruct2,
)


def _free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        sock.listen(1)
        return int(sock.getsockname()[1])


def _wait_until(predicate, timeout: float = 1.0) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.01)
    assert predicate()


def _wait_event(event: threading.Event, timeout: float = 1.0) -> None:
    assert event.wait(timeout), "timed out waiting for callback"


@pytest.fixture
def server_bundle():
    errors: list[Exception] = []
    server = StatesServer(port=_free_port(), error_handler=errors.append)
    server.start()
    try:
        yield server, server.states, errors
    finally:
        if server.is_running():
            server.stop()


def test_server_lifecycle_and_scalar_roundtrips(server_bundle) -> None:
    server, states, _errors = server_bundle

    assert server.is_running()
    assert not server.is_connected()

    states.scalars.bool_value.set(True)
    states.scalars.count.set(41)
    states.scalars.ratio.set(0.75)
    states.scalars.queued_progress.set(0.5)
    states.scalars.title.set("title value")
    states.scalars.optional_value.set(13)
    states.scalars.fixed_numbers.set([3, 5, 8])
    states.scalars.test_enum.set(ExampleTestEnum.C)

    assert states.scalars.bool_value.get() is True
    assert states.scalars.count.get() == 41
    assert states.scalars.ratio.get() == pytest.approx(0.75)
    assert states.scalars.queued_progress.get() == pytest.approx(0.5)
    assert states.scalars.title.get() == "title value"
    assert states.scalars.optional_value.get() == 13
    assert states.scalars.fixed_numbers.get() == [3, 5, 8]
    assert states.scalars.test_enum.get() == ExampleTestEnum.C

    point = ExampleTestStruct(1.25, -4.5, "origin")
    optional_struct = ExampleTestStruct2(True, 6, "nested")
    states.custom.point.set(point)
    states.custom.choice.set(ExampleTestEnum2.Z)
    states.custom.optional_struct.set(optional_struct)
    states.nested.counter.set(11)
    states.nested.inner.selected.set(ExampleTestEnum.B)
    states.nested.inner.leaf.enabled.set(True)
    states.nested.inner.leaf.message.set("leaf message")

    assert states.custom.point.get() == point
    assert states.custom.choice.get() == ExampleTestEnum2.Z
    assert states.custom.optional_struct.get() == optional_struct
    assert states.nested.counter.get() == 11
    assert states.nested.inner.selected.get() == ExampleTestEnum.B
    assert states.nested.inner.leaf.enabled.get() is True
    assert states.nested.inner.leaf.message.get() == "leaf message"


def test_static_collection_image_and_data_methods(server_bundle) -> None:
    _server, states, _errors = server_bundle

    summary = ExampleTestStruct2(True, 9, "summary")
    states.statics.status_text.set("static text")
    states.statics.summary.set(summary)
    states.statics.pair.set([1.5, 2.5])

    assert states.statics.status_text.get() == "static text"
    assert states.statics.summary.get() == summary
    assert states.statics.pair.get() == [1.5, 2.5]

    states.collections.plain_vec_value.set([4, 8, 15, 16, 23, 42])
    assert states.collections.plain_vec_value.get() == [4, 8, 15, 16, 23, 42]

    states.collections.list.set([1, 2, 3])
    states.collections.list.set_item(1, 20)
    states.collections.list.add_item(99)
    assert states.collections.list.get_item(1) == 20
    assert states.collections.list[0] == 1
    states.collections.list[0] = 7
    states.collections.list.remove_item(2)
    assert states.collections.list.get() == [7, 20, 99]

    states.collections.map.set({1: 100, 2: 200})
    states.collections.map.set_item(5, 500)
    assert states.collections.map.get_item(1) == 100
    assert states.collections.map[2] == 200
    states.collections.map[2] = 220
    del states.collections.map[1]
    assert states.collections.map.get() == {2: 220, 5: 500}

    image = np.zeros((8, 8, 4), dtype=np.uint8)
    image[..., 0] = 10
    image[..., 3] = 255
    states.data.image.set(image)

    image_result = states.data.image.get()
    assert states.data.image.shape() == (8, 8)
    assert image_result.shape == (8, 8, 4)
    assert image_result[0, 0, 0] == 10
    assert image_result[0, 7, 0] == 10
    assert image_result[7, 0, 0] == 10
    assert image_result[7, 7, 0] == 10
    assert np.all(image_result[..., 3] == 255)

    bytes_data = np.arange(6, dtype=np.uint8)
    states.data.bytes.set(bytes_data)
    states.data.bytes.add(np.array([6, 7], dtype=np.uint8))
    states.data.bytes.replace(np.array([50, 51], dtype=np.uint8), 2)
    states.data.bytes.remove(1, 2)
    np.testing.assert_array_equal(
        states.data.bytes.get(),
        np.array([0, 51, 4, 5, 6, 7], dtype=np.uint8),
    )
    states.data.bytes.clear()
    np.testing.assert_array_equal(states.data.bytes.get(), np.array([], dtype=np.uint8))

    samples = np.linspace(0.0, 1.0, 5, dtype=np.float32)
    states.data.samples.set(samples)
    np.testing.assert_allclose(states.data.samples.get(), samples)

    nested_buffer = np.arange(4, dtype=np.uint16)
    states.nested.inner.leaf.buffer.set(nested_buffer)
    states.nested.inner.leaf.buffer.add(np.array([4, 5], dtype=np.uint16))
    np.testing.assert_array_equal(
        states.nested.inner.leaf.buffer.get(),
        np.array([0, 1, 2, 3, 4, 5], dtype=np.uint16),
    )

    states.events.take_text.set("queued take", update=True)
    states.events.take_empty.set(update=True)


def test_value_callbacks_disconnect_and_signal_mode(server_bundle) -> None:
    _server, states, _errors = server_bundle

    title_values: list[str] = []
    title_event = threading.Event()

    def on_title(value: str) -> None:
        title_values.append(value)
        if len(title_values) >= 2:
            title_event.set()

    states.scalars.title.connect(on_title)
    states.scalars.title.signal_set_to_queue()
    states.scalars.title.set("first", set_signal=True)
    states.scalars.title.set("second", set_signal=True)
    _wait_event(title_event)
    assert title_values[:2] == ["first", "second"]

    states.scalars.title.disconnect(on_title)
    title_event.clear()
    states.scalars.title.set("third", set_signal=True)
    assert not title_event.wait(0.2)

    ratio_values: list[float] = []
    ratio_event = threading.Event()

    def on_ratio(value: float) -> None:
        ratio_values.append(value)
        ratio_event.set()

    states.scalars.ratio.connect(on_ratio)
    states.scalars.ratio.signal_set_to_single()
    states.scalars.ratio.set(0.9, set_signal=True)
    _wait_event(ratio_event)
    assert ratio_values[-1] == pytest.approx(0.9)

    states.scalars.ratio.disconnect_all()
    ratio_event.clear()
    states.scalars.ratio.set(0.1, set_signal=True)
    assert not ratio_event.wait(0.2)


def test_signal_callbacks_and_disconnect_all(server_bundle) -> None:
    _server, states, _errors = server_bundle

    signal_events: list[str] = []
    empty_event = threading.Event()
    number_event = threading.Event()
    enum_event = threading.Event()

    def on_empty() -> None:
        signal_events.append("empty")
        empty_event.set()

    def on_number(value: float) -> None:
        signal_events.append(f"number:{value}")
        number_event.set()

    def on_enum(value: ExampleTestEnum) -> None:
        signal_events.append(f"enum:{value.name}")
        enum_event.set()

    states.events.empty_signal.connect(on_empty)
    states.events.number_signal.connect(on_number)
    states.events.enum_signal.connect(on_enum)

    states.events.empty_signal.signal_set_to_queue()
    states.events.empty_signal.set()
    states.events.number_signal.set(1.25)
    states.events.enum_signal.set(ExampleTestEnum.B)

    _wait_event(empty_event)
    _wait_event(number_event)
    _wait_event(enum_event)
    assert "empty" in signal_events
    assert "number:1.25" in signal_events
    assert "enum:B" in signal_events

    states.events.enum_signal.disconnect_all()
    enum_event.clear()
    states.events.enum_signal.set(ExampleTestEnum.C)
    assert not enum_event.wait(0.2)


def test_error_handler_receives_callback_failures() -> None:
    captured_errors: list[Exception] = []
    error_event = threading.Event()

    def on_error(error: Exception) -> None:
        captured_errors.append(error)
        error_event.set()

    server = StatesServer(port=_free_port(), error_handler=on_error)
    server.start()
    try:

        def explode(_value: float) -> None:
            raise ValueError("boom")

        server.states.events.number_signal.connect(explode)
        server.states.events.number_signal.set(3.5)
        _wait_event(error_event)
        _wait_until(lambda: bool(captured_errors))
        assert isinstance(captured_errors[0], ValueError)
        assert str(captured_errors[0]) == "boom"
    finally:
        if server.is_running():
            server.stop()
