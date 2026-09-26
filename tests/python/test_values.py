"""Local conversion checks have no listener or callback workers."""

from operator import attrgetter
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
        ("fixed_numbers", [1, 2]),
        ("fixed_numbers", [1, 2, 3, 4]),
        ("fixed_numbers", [1, "wrong", 3]),
        ("count", 2**31),
        ("count", -(2**31) - 1),
        ("count", "wrong"),
        ("title", 17),
    ],
)
def test_invalid_value_inputs_preserve_previous_value(local_bundle, field, invalid):
    _, states, _ = local_bundle
    value = getattr(states.values, field)
    before = value.get()
    with pytest.raises((ValueError, TypeError, OverflowError)):
        value.set(invalid)
    assert value.get() == before
