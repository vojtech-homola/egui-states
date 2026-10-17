"""Local conversion checks have no listener or callback workers."""

from operator import attrgetter
import threading

import numpy as np
import pytest
from egui_states_test_bindings.enums import TestEnum as Choice, TestEnum2 as Secondary
from egui_states_test_bindings.structs import TestStruct as Point, TestStruct2 as Summary


@pytest.mark.parametrize(
    "field,value",
    [
        ("values.bool_value", True),
        ("values.count", 41),
        ("values.ratio", 0.75),
        ("values.queued_progress", 0.5),
        ("values.title", "title value"),
        ("values.optional_value", 13),
        ("values.optional_value", None),
        ("values.fixed_numbers", [3, 5, 8]),
        ("values.test_enum", Choice.C),
        ("values.nested.secondary_choice", Secondary.Z),
        ("values.nested.selected_enum", Choice.B),
        ("values.nested.selected_enum", None),
        ("custom_values.point", Point(1.25, -4.5, "origin")),
        ("custom_values.optional_struct", Summary(True, 6, "nested")),
        ("custom_values.optional_struct", None),
        ("statics.status_text", "static text"),
        ("statics.summary", Summary(True, 9, "summary")),
        ("statics.pair", [1.5, 2.5]),
        ("statics.nested.label", "nested static"),
        ("statics.nested.enum_hint", Choice.B),
        ("wire_types.small_unsigned", 255),
        ("wire_types.small_signed", -128),
        ("wire_types.short", -32768),
        ("wire_types.wide", 2**64 - 1),
        ("wire_types.single", -0.125),
        ("wire_types.tuple", (-7, "tuple", True)),
        ("wire_types.list", [Summary(True, 1, "a"), Summary(False, 2, "b")]),
        ("wire_types.list", []),
        ("wire_types.table", {"pi": 3.5, "neg": -0.5}),
        ("wire_types.keyed_enum", {Choice.A: 1, Choice.C: 3}),
        ("wire_types.optional_enum", Secondary.Z),
        ("wire_types.optional_enum", None),
        ("wire_types.bytes", [0, 127, 255]),
    ],
    ids=lambda value: value if isinstance(value, str) else None,
)
def test_local_value_conversion(local_bundle, field, value):
    server, states, _ = local_bundle
    assert not server.is_running()
    assert not server.is_connected()
    handle = attrgetter(field)(states)
    handle.set(value)
    assert handle.get() == value


@pytest.mark.parametrize(
    "field,invalid",
    [
        ("values.fixed_numbers", [1, 2]),
        ("values.fixed_numbers", [1, 2, 3, 4]),
        ("values.fixed_numbers", [1, "wrong", 3]),
        ("values.count", 2**31),
        ("values.count", -(2**31) - 1),
        ("values.count", "wrong"),
        ("values.title", 17),
        ("values.test_enum", 2),
        ("wire_types.small_unsigned", 256),
        ("wire_types.small_signed", -129),
        ("wire_types.wide", -1),
        ("wire_types.tuple", (1, "short")),
        ("wire_types.tuple", (1, "long", True, False)),
        ("wire_types.table", {1: 2.0}),
    ],
    ids=lambda value: value if isinstance(value, str) else None,
)
def test_invalid_value_inputs_preserve_previous_value(local_bundle, field, invalid):
    _, states, _ = local_bundle
    value = attrgetter(field)(states)
    before = value.get()
    with pytest.raises((ValueError, TypeError, OverflowError)):
        value.set(invalid)
    assert value.get() == before


def test_single_precision_values_are_rounded_to_f32(local_bundle):
    """Pins current behavior: f32 states store the nearest f32, not the Python float."""
    _, states, _ = local_bundle
    states.wire_types.single.set(0.1)
    assert states.wire_types.single.get() == float(np.float32(0.1))
    assert states.wire_types.single.get() != 0.1


@pytest.mark.parametrize("kind", ["value", "empty", "data", "sparse"])
def test_blocking_takes_without_a_client_return_immediately(local_bundle, kind):
    """A blocking send waits for the client's acknowledgement; without a client it must not wait."""
    _, states, _ = local_bundle
    st = states.integration
    send = {
        "value": lambda: st.take.set("payload", blocking=True),
        "empty": lambda: st.empty.set(blocking=True),
        "data": lambda: st.data.set(np.array([1, 2], dtype=np.uint8), blocking=True),
        "sparse": lambda: st.multi[3].set(np.array([1, 2], dtype=np.uint16), blocking=True),
    }[kind]
    errors = []

    def attempt():
        try:
            for _ in range(2):  # The second send would wait for the first one.
                send()
        except Exception as error:  # noqa: BLE001 - reported below
            errors.append(error)

    # The native call releases the GIL, so a regression leaves only a daemon thread behind.
    worker = threading.Thread(target=attempt, daemon=True)
    worker.start()
    worker.join(5)
    assert not worker.is_alive(), f"blocking {kind} take waited without a client"
    assert errors == []
