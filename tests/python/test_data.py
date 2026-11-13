# ruff: noqa: D103
"""Numeric buffer mutation and range validation through generated state handles."""

import threading

import numpy as np
import pytest
from .assertions import assert_array

from egui_states_test_bindings import (  # ty: ignore[import-not-found]
    State,
    StatesServer,
)
from egui_states_test_bindings.structs import HeaderMeta  # ty: ignore[import-not-found]


def test_data_array_methods(local_bundle: tuple[StatesServer, State, list[Exception]]) -> None:
    _server, states, _errors = local_bundle

    bytes_data = np.arange(6, dtype=np.uint8)
    states.data.bytes.set(bytes_data)
    states.data.bytes.add(np.array([6, 7], dtype=np.uint8))
    states.data.bytes.replace(np.array([50, 51], dtype=np.uint8), 2)
    states.data.bytes.remove(1, 2)
    assert_array(
        states.data.bytes.get(),
        np.array([0, 51, 4, 5, 6, 7], dtype=np.uint8),
    )
    states.data.bytes.clear()
    assert_array(states.data.bytes.get(), np.array([], dtype=np.uint8))


def test_nested_numeric_storage(local_bundle):
    _, states, _ = local_bundle
    nested_buffer = np.arange(4, dtype=np.uint16)
    states.data.nested.buffer.set(nested_buffer)
    states.data.nested.buffer.add(np.array([4, 5], dtype=np.uint16))
    assert_array(
        states.data.nested.buffer.get(),
        np.array([0, 1, 2, 3, 4, 5], dtype=np.uint16),
    )


def test_multi_data_methods(local_bundle: tuple[StatesServer, State, list[Exception]]) -> None:
    _server, states, _errors = local_bundle

    bytes_zero = states.multi_data.bytes[0]
    bytes_zero.set(np.array([0, 1, 2, 3], dtype=np.uint8))
    bytes_zero.add(np.array([4, 5], dtype=np.uint8))
    bytes_zero.replace(np.array([50, 51], dtype=np.uint8), 2)
    bytes_zero.remove(1, 2)
    assert_array(bytes_zero.get(), np.array([0, 51, 4, 5], dtype=np.uint8))
    bytes_zero.clear()
    assert_array(bytes_zero.get(), np.array([], dtype=np.uint8))

    bytes_one = states.multi_data.bytes.get(1)
    bytes_one.set(np.array([9, 8, 7], dtype=np.uint8))
    bytes_one.add(np.array([6], dtype=np.uint8))
    assert_array(bytes_one.get(), np.array([9, 8, 7, 6], dtype=np.uint8))
    states.multi_data.bytes.remove_index(1)
    with pytest.raises(ValueError, match="DataMulti index not found"):
        states.multi_data.bytes[1].get()

    states.multi_data.bytes[2].set(np.array([42, 43], dtype=np.uint8))
    states.multi_data.bytes.remove_index(2)
    with pytest.raises(ValueError, match="DataMulti index not found"):
        states.multi_data.bytes[2].get()


def test_sparse_float_storage(local_bundle):
    _, states, _ = local_bundle
    samples = np.linspace(0.0, 1.0, 5, dtype=np.float32)
    states.multi_data.samples[3].set(samples)
    states.multi_data.samples[3].add(np.array([1.25], dtype=np.float32))
    assert_array(
        states.multi_data.samples[3].get(),
        np.array([0.0, 0.25, 0.5, 0.75, 1.0, 1.25], dtype=np.float32),
    )


def test_nested_sparse_numeric_storage(local_bundle):
    _, states, _ = local_bundle
    nested_buffer = states.multi_data.nested.buffer[4]
    nested_buffer.set(np.array([0, 1, 2, 3], dtype=np.uint16))
    nested_buffer.replace(np.array([10, 11], dtype=np.uint16), 1)
    nested_buffer.add(np.array([12], dtype=np.uint16))
    assert_array(
        nested_buffer.get(),
        np.array([0, 10, 11, 3, 12], dtype=np.uint16),
    )


def test_sparse_numeric_reset(local_bundle):
    _, states, _ = local_bundle
    states.multi_data.bytes[0].set(np.array([8], dtype=np.uint8))
    states.multi_data.bytes[5].set(np.array([1, 2, 3], dtype=np.uint8))
    states.multi_data.bytes.reset()
    with pytest.raises(ValueError, match="DataMulti index not found"):
        states.multi_data.bytes[0].get()
    with pytest.raises(ValueError, match="DataMulti index not found"):
        states.multi_data.bytes[5].get()


def test_sparse_numeric_reset_and_invalid_mutations(local_bundle):
    _, states, _ = local_bundle
    data = states.multi_data.bytes
    for key in [0, 1, 2**32 - 1]:
        data[key].set(np.array([1, 2, 3], dtype=np.uint8))
    for operation in ("replace", "remove"):
        with pytest.raises(ValueError):
            if operation == "replace":
                data[1].replace(np.array([9], dtype=np.uint8), 4)
            else:
                data[1].remove(2, 2)
        assert_array(data[1].get(), np.array([1, 2, 3], dtype=np.uint8))
    data.remove_index(777)  # missing removal is harmless
    data.reset()
    for key in [0, 1, 2**32 - 1]:
        with pytest.raises(ValueError, match="index not found"):
            data[key].get()


def test_single_buffer_invalid_ranges_preserve_data(local_bundle):
    _, states, _ = local_bundle
    data = states.data.bytes
    data.set(np.array([1, 2, 3], dtype=np.uint8))
    for name, operation in [
        ("replace past the end", lambda: data.replace(np.array([9, 9], dtype=np.uint8), 2)),
        ("remove past the end", lambda: data.remove(2, 2)),
        ("remove nothing", lambda: data.remove(0, 0)),
    ]:
        with pytest.raises(ValueError):
            operation()
        assert data.get().tolist() == [1, 2, 3], name


@pytest.mark.parametrize("operation", ["replace", "remove", "multi replace", "multi remove"])
def test_huge_data_indexes_raise_value_error(local_bundle, operation):
    """Range checks must not overflow into a native panic."""
    _, states, _ = local_bundle
    data, multi = states.data.bytes, states.multi_data.bytes
    data.set(np.array([1, 2, 3], dtype=np.uint8))
    multi[4].set(np.array([1, 2, 3], dtype=np.uint8))
    huge = 2**64 - 1
    with pytest.raises(ValueError):
        {
            "replace": lambda: data.replace(np.array([9], dtype=np.uint8), huge),
            "remove": lambda: data.remove(huge, 1),
            "multi replace": lambda: multi[4].replace(np.array([9], dtype=np.uint8), huge),
            "multi remove": lambda: multi[4].remove(huge, 1),
        }[operation]()
    assert_array(data.get(), np.array([1, 2, 3], dtype=np.uint8))
    assert_array(multi[4].get(), np.array([1, 2, 3], dtype=np.uint8))


def test_data_is_not_set_until_a_set_creates_it(local_bundle):
    _, states, _ = local_bundle
    data = states.data.bytes
    assert data.get() is None
    changes = {
        "add": lambda: data.add(np.array([1], dtype=np.uint8)),
        "replace": lambda: data.replace(np.array([], dtype=np.uint8), 0),
        "remove": lambda: data.remove(0, 1),
        "clear": data.clear,
    }
    for name, change in changes.items():
        with pytest.raises(ValueError, match="is not set"):
            change()
        assert data.get() is None, name
    data.reset()

    # Empty data is set, unlike no data.
    data.set(np.array([], dtype=np.uint8))
    assert_array(data.get(), np.array([], dtype=np.uint8))
    data.add(np.array([4], dtype=np.uint8))
    assert_array(data.get(), np.array([4], dtype=np.uint8))
    data.reset()
    assert data.get() is None


def test_data_headers_are_stored_with_their_buffer(local_bundle):
    _, states, _ = local_bundle
    series = states.headers.series
    assert series.get_header() is None
    assert series.get_with_header() is None
    # The set that creates the data needs a header, and a header needs data.
    with pytest.raises(ValueError, match="needs a header"):
        series.set(np.array([1.0], dtype=np.float32))
    with pytest.raises(ValueError, match="is not set"):
        series.set_header(HeaderMeta(1.0, "z", []))
    series.set(np.array([1.0], dtype=np.float32), header=HeaderMeta(1.5, "a", [1]))
    series.add(np.array([2.0], dtype=np.float32))
    assert series.get_header() == HeaderMeta(1.5, "a", [1])
    data, header = series.get_with_header()
    assert_array(data, np.array([1.0, 2.0], dtype=np.float32))
    assert header == HeaderMeta(1.5, "a", [1])
    series.clear(header=HeaderMeta(2.0, "b", []))
    series.set_header(HeaderMeta(3.0, "c", [2, 3]))
    assert_array(series.get(), np.array([], dtype=np.float32))
    assert series.get_header() == HeaderMeta(3.0, "c", [2, 3])
    series.reset()
    assert series.get_with_header() is None
    assert series.get_header() is None

    keyed = states.headers.keyed
    with pytest.raises(ValueError, match="needs a header"):
        keyed[1].set(np.array([7], dtype=np.uint16))
    keyed[1].set(np.array([7], dtype=np.uint16), header=HeaderMeta(0.5, "y", []))
    keyed[1].set(np.array([6], dtype=np.uint16))
    assert keyed[1].get_header() == HeaderMeta(0.5, "y", [])
    assert keyed[1].get_with_header()[1] == HeaderMeta(0.5, "y", [])
    keyed[1].replace(np.array([8], dtype=np.uint16), 0, header=HeaderMeta(4.0, "d", []))
    assert keyed[1].get_header() == HeaderMeta(4.0, "d", [])
    data, header = keyed[1].get_with_header()
    assert_array(data, np.array([8], dtype=np.uint16))
    assert header == HeaderMeta(4.0, "d", [])
    with pytest.raises(ValueError, match="does not exist"):
        keyed[2].set_header(HeaderMeta(5.0, "e", []))
    with pytest.raises(ValueError, match="index not found"):
        keyed[2].get_header()
    with pytest.raises(ValueError, match="index not found"):
        keyed[2].get_with_header()
    keyed.remove_index(1)
    with pytest.raises(ValueError, match="index not found"):
        keyed[1].get_header()

    # Every take carries a header.
    take, keyed_take = states.headers.take, states.headers.keyed_take
    with pytest.raises(ValueError, match="needs a header"):
        take.set(np.array([1], dtype=np.uint8))
    with pytest.raises(ValueError, match="needs a header"):
        keyed_take[3].set(np.array([1], dtype=np.uint8))
    take.set(np.array([1], dtype=np.uint8), header=HeaderMeta(6.0, "f", []))
    keyed_take[3].set(np.array([1], dtype=np.uint8), header=HeaderMeta(7.0, "g", []))


@pytest.mark.parametrize("with_buffer", [False, True])
def test_a_header_is_decoded_outside_the_storage_lock(local_bundle, monkeypatch, with_buffer):
    """Decoding runs the header class's Python code, which may use the state.

    While the storage lock was held during decoding, a header class that wrote
    to the same state deadlocked; so did a second Python thread reading it
    while the connection waited to write.
    """
    _, states, _ = local_bundle
    series = states.headers.series
    keyed = states.headers.keyed
    series.set(np.array([1.0], dtype=np.float32), header=HeaderMeta(1.0, "a", []))
    keyed[1].set(np.array([1], dtype=np.uint16), header=HeaderMeta(1.0, "a", []))

    init = HeaderMeta.__init__
    writes = []

    def writing_init(self, *args, **kwargs):
        init(self, *args, **kwargs)
        if writes:
            write = writes.pop()
            write()

    monkeypatch.setattr(HeaderMeta, "__init__", writing_init)

    def read(get_header, write):
        writes.append(write)
        result = []
        worker = threading.Thread(target=lambda: result.append(get_header()), daemon=True)
        worker.start()
        worker.join(timeout=5)
        assert not worker.is_alive(), "decoding a header deadlocked"
        return result[0]

    def write_series():
        series.set_header(HeaderMeta(2.0, "b", []))

    def write_keyed():
        keyed[1].set_header(HeaderMeta(2.0, "b", []))

    def getter(single):
        return (lambda: single.get_with_header()[1]) if with_buffer else single.get_header

    assert read(getter(series), write_series) == HeaderMeta(1.0, "a", [])
    assert read(getter(keyed[1]), write_keyed) == HeaderMeta(1.0, "a", [])

    monkeypatch.undo()
    assert series.get_header() == HeaderMeta(2.0, "b", [])
    assert keyed[1].get_header() == HeaderMeta(2.0, "b", [])


def test_a_header_of_the_wrong_type_changes_nothing(local_bundle):
    _, states, _ = local_bundle
    series = states.headers.series
    with pytest.raises((TypeError, ValueError)):
        series.set(np.array([2.0, 3.0], dtype=np.float32), header="not a header")
    assert series.get_with_header() is None
    series.set(np.array([1.0], dtype=np.float32), header=HeaderMeta(1.0, "a", []))
    with pytest.raises((TypeError, ValueError)):
        series.set(np.array([2.0, 3.0], dtype=np.float32), header="not a header")
    assert_array(series.get(), np.array([1.0], dtype=np.float32))
    assert series.get_header() == HeaderMeta(1.0, "a", [])
