# ruff: noqa: D103, E402, PLR0915
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
    States,
    TestEnum as ExampleTestEnum,
    TestEnum2 as ExampleTestEnum2,
    TestStruct as ExampleTestStruct,
    TestStruct2 as ExampleTestStruct2,
)

DEFAULT_VEC = [10, -3, 27]
DEFAULT_MAP = {1: 100, 2: 200, 5: 500}


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


def _wire_collection_actions(states) -> None:
    def reset_vec() -> None:
        states.value_vec.items.set(list(DEFAULT_VEC), update=True)

    def append_vec() -> None:
        current = states.value_vec.items.get()
        next_value = current[-1] + 5 if current else DEFAULT_VEC[0]
        states.value_vec.items.add_item(next_value, update=True)

    def remove_last_vec() -> None:
        current = states.value_vec.items.get()
        if current:
            states.value_vec.items.remove_item(len(current) - 1, update=True)

    def reset_map() -> None:
        states.value_map.items.set(dict(DEFAULT_MAP), update=True)

    def insert_next_map() -> None:
        current = states.value_map.items.get()
        next_key = max(current, default=0) + 1
        states.value_map.items.set_item(next_key, next_key * 100, update=True)

    def remove_lowest_map() -> None:
        current = states.value_map.items.get()
        if current:
            states.value_map.items.remove_item(min(current), update=True)

    states.value_vec.actions.append_item.connect(append_vec)
    states.value_vec.actions.remove_last.connect(remove_last_vec)
    states.value_vec.actions.reset_demo.connect(reset_vec)

    states.value_map.actions.insert_next.connect(insert_next_map)
    states.value_map.actions.remove_lowest.connect(remove_lowest_map)
    states.value_map.actions.reset_demo.connect(reset_map)


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


def test_server_lifecycle_and_value_roundtrips(server_bundle: tuple[StatesServer, States, list[Exception]]) -> None:
    server, states, _errors = server_bundle

    assert server.is_running()
    assert not server.is_connected()

    states.values.bool_value.set(True)
    states.values.count.set(41)
    states.values.ratio.set(0.75)
    states.values.queued_progress.set(0.5)
    states.values.title.set("title value")
    states.values.optional_value.set(13)
    states.values.fixed_numbers.set([3, 5, 8])
    states.values.test_enum.set(ExampleTestEnum.C)
    states.values.nested.secondary_choice.set(ExampleTestEnum2.Z)
    states.values.nested.selected_enum.set(ExampleTestEnum.B)

    assert states.values.bool_value.get() is True
    assert states.values.count.get() == 41
    assert states.values.ratio.get() == pytest.approx(0.75)
    assert states.values.queued_progress.get() == pytest.approx(0.5)
    assert states.values.title.get() == "title value"
    assert states.values.optional_value.get() == 13
    assert states.values.fixed_numbers.get() == [3, 5, 8]
    assert states.values.test_enum.get() == ExampleTestEnum.C
    assert states.values.nested.secondary_choice.get() == ExampleTestEnum2.Z
    assert states.values.nested.selected_enum.get() == ExampleTestEnum.B

    point = ExampleTestStruct(1.25, -4.5, "origin")
    optional_struct = ExampleTestStruct2(True, 6, "nested")
    states.custom_values.point.set(point)
    states.custom_values.optional_struct.set(optional_struct)

    assert states.custom_values.point.get() == point
    assert states.custom_values.optional_struct.get() == optional_struct


def test_static_value_roundtrips(server_bundle: tuple[StatesServer, States, list[Exception]]) -> None:
    _server, states, _errors = server_bundle

    summary = ExampleTestStruct2(True, 9, "summary")
    states.statics.status_text.set("static text")
    states.statics.summary.set(summary)
    states.statics.pair.set([1.5, 2.5])
    states.statics.nested.label.set("nested static")
    states.statics.nested.enum_hint.set(ExampleTestEnum.B)

    assert states.statics.status_text.get() == "static text"
    assert states.statics.summary.get() == summary
    assert states.statics.pair.get() == pytest.approx([1.5, 2.5])
    assert states.statics.nested.label.get() == "nested static"
    assert states.statics.nested.enum_hint.get() == ExampleTestEnum.B


def test_image_value_roundtrip(server_bundle: tuple[StatesServer, States, list[Exception]]) -> None:
    _server, states, _errors = server_bundle

    image = np.zeros((8, 8, 4), dtype=np.uint8)
    image[..., 0] = 10
    image[..., 3] = 255
    states.image.image.set(image)

    image_result = states.image.image.get()
    assert states.image.image.shape() == (8, 8)
    assert image_result.shape == (8, 8, 4)
    assert image_result[0, 0, 0] == 10
    assert np.all(image_result[..., 3] == 255)


def test_data_array_methods(server_bundle: tuple[StatesServer, States, list[Exception]]) -> None:
    _server, states, _errors = server_bundle

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
    states.data.nested.buffer.set(nested_buffer)
    states.data.nested.buffer.add(np.array([4, 5], dtype=np.uint16))
    np.testing.assert_array_equal(
        states.data.nested.buffer.get(),
        np.array([0, 1, 2, 3, 4, 5], dtype=np.uint16),
    )


def test_multi_data_methods(server_bundle: tuple[StatesServer, States, list[Exception]]) -> None:
    _server, states, _errors = server_bundle

    bytes_zero = states.multi_data.bytes[0]
    bytes_zero.set(np.array([0, 1, 2, 3], dtype=np.uint8))
    bytes_zero.add(np.array([4, 5], dtype=np.uint8))
    bytes_zero.replace(np.array([50, 51], dtype=np.uint8), 2)
    bytes_zero.remove(1, 2)
    np.testing.assert_array_equal(bytes_zero.get(), np.array([0, 51, 4, 5], dtype=np.uint8))
    bytes_zero.clear()
    np.testing.assert_array_equal(bytes_zero.get(), np.array([], dtype=np.uint8))

    bytes_one = states.multi_data.bytes.get(1)
    bytes_one.set(np.array([9, 8, 7], dtype=np.uint8))
    bytes_one.add(np.array([6], dtype=np.uint8))
    np.testing.assert_array_equal(bytes_one.get(), np.array([9, 8, 7, 6], dtype=np.uint8))
    states.multi_data.bytes.remove_index(1)
    with pytest.raises(ValueError, match="DataMulti index not found"):
        states.multi_data.bytes[1].get()

    states.multi_data.bytes[2].set(np.array([42, 43], dtype=np.uint8))
    states.multi_data.bytes.remove_index(2)
    with pytest.raises(ValueError, match="DataMulti index not found"):
        states.multi_data.bytes[2].get()

    samples = np.linspace(0.0, 1.0, 5, dtype=np.float32)
    states.multi_data.samples[3].set(samples)
    states.multi_data.samples[3].add(np.array([1.25], dtype=np.float32))
    np.testing.assert_allclose(
        states.multi_data.samples[3].get(),
        np.array([0.0, 0.25, 0.5, 0.75, 1.0, 1.25], dtype=np.float32),
    )

    nested_buffer = states.multi_data.nested.buffer[4]
    nested_buffer.set(np.array([0, 1, 2, 3], dtype=np.uint16))
    nested_buffer.replace(np.array([10, 11], dtype=np.uint16), 1)
    nested_buffer.add(np.array([12], dtype=np.uint16))
    np.testing.assert_array_equal(
        nested_buffer.get(),
        np.array([0, 10, 11, 3, 12], dtype=np.uint16),
    )

    states.multi_data.bytes[5].set(np.array([1, 2, 3], dtype=np.uint8))
    states.multi_data.bytes.reset()
    with pytest.raises(ValueError, match="DataMulti index not found"):
        states.multi_data.bytes[0].get()
    with pytest.raises(ValueError, match="DataMulti index not found"):
        states.multi_data.bytes[5].get()


def test_value_take_methods(server_bundle: tuple[StatesServer, States, list[Exception]]) -> None:
    _server, states, _errors = server_bundle

    states.value_take.take_text.set("queued take", update=True)
    states.value_take.take_empty.set(update=True)


def test_value_vec_and_value_map_follow_action_signals(
    server_bundle: tuple[StatesServer, States, list[Exception]],
) -> None:
    _server, states, _errors = server_bundle
    _wire_collection_actions(states)

    states.value_vec.actions.reset_demo.set()
    _wait_until(lambda: states.value_vec.items.get() == DEFAULT_VEC)

    states.value_vec.actions.append_item.set()
    _wait_until(lambda: states.value_vec.items.get() == [10, -3, 27, 32])

    states.value_vec.actions.remove_last.set()
    _wait_until(lambda: states.value_vec.items.get() == DEFAULT_VEC)

    states.value_map.actions.reset_demo.set()
    _wait_until(lambda: states.value_map.items.get() == DEFAULT_MAP)

    states.value_map.actions.insert_next.set()
    _wait_until(lambda: states.value_map.items.get() == {1: 100, 2: 200, 5: 500, 6: 600})

    states.value_map.actions.remove_lowest.set()
    _wait_until(lambda: states.value_map.items.get() == {2: 200, 5: 500, 6: 600})


def test_value_callbacks_disconnect_and_signal_mode(
    server_bundle: tuple[StatesServer, States, list[Exception]],
) -> None:
    _server, states, _errors = server_bundle

    title_values: list[str] = []
    title_event = threading.Event()

    def on_title(value: str) -> None:
        title_values.append(value)
        if len(title_values) >= 2:
            title_event.set()

    states.values.title.connect(on_title)
    states.values.title.signal_set_to_queue()
    states.values.title.set("first", set_signal=True)
    states.values.title.set("second", set_signal=True)
    _wait_event(title_event)
    assert title_values[:2] == ["first", "second"]

    states.values.title.disconnect(on_title)
    title_event.clear()
    states.values.title.set("third", set_signal=True)
    assert not title_event.wait(0.2)

    ratio_values: list[float] = []
    ratio_event = threading.Event()

    def on_ratio(value: float) -> None:
        ratio_values.append(value)
        ratio_event.set()

    states.values.ratio.connect(on_ratio)
    states.values.ratio.signal_set_to_single()
    states.values.ratio.set(0.9, set_signal=True)
    _wait_event(ratio_event)
    assert ratio_values[-1] == pytest.approx(0.9)

    states.values.ratio.disconnect_all()
    ratio_event.clear()
    states.values.ratio.set(0.1, set_signal=True)
    assert not ratio_event.wait(0.2)


def test_signal_callbacks_and_disconnect_all(
    server_bundle: tuple[StatesServer, States, list[Exception]],
) -> None:
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

    states.signals.empty_signal.connect(on_empty)
    states.signals.number_signal.connect(on_number)
    states.signals.enum_signal.connect(on_enum)

    states.signals.empty_signal.signal_set_to_queue()
    states.signals.empty_signal.set()
    states.signals.number_signal.set(1.25)
    states.signals.enum_signal.set(ExampleTestEnum.B)

    _wait_event(empty_event)
    _wait_event(number_event)
    _wait_event(enum_event)
    assert "empty" in signal_events
    assert "number:1.25" in signal_events
    assert "enum:B" in signal_events

    states.signals.enum_signal.disconnect_all()
    enum_event.clear()
    states.signals.enum_signal.set(ExampleTestEnum.C)
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

        server.states.signals.number_signal.connect(explode)
        server.states.signals.number_signal.set(3.5)
        _wait_event(error_event)
        _wait_until(lambda: bool(captured_errors))
        assert isinstance(captured_errors[0], ValueError)
        assert str(captured_errors[0]) == "boom"
    finally:
        if server.is_running():
            server.stop()


def test_data_take_methods(server_bundle: tuple[StatesServer, States, list[Exception]]) -> None:
    _server, states, _errors = server_bundle

    buffer_data = np.array([0, 1, 2, 3, 4], dtype=np.uint8)
    states.data_take.take_buffer.set(buffer_data, blocking=True, update=True)

    samples = np.linspace(0.0, 1.0, 5, dtype=np.float32)
    states.data_take.take_samples.set(samples, blocking=False, update=True)
