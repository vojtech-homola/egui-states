# ruff: noqa: PLC0415
"""Python server implementation for the shared native-client scenarios."""

import numpy as np
from egui_states_test_bindings import StatesServer  # ty: ignore[unresolved-import]
from tests.preparation import run
from tests.python_test_helpers.network import free_port
from tests.python_test_helpers.waiting import wait_until


def set_wire_values(states):
    """Command 30: non-default values of every shape for the client to check."""
    from egui_states_test_bindings.enums import TestEnum, TestEnum2  # ty: ignore[unresolved-import]
    from egui_states_test_bindings.structs import TestStruct, TestStruct2  # ty: ignore[unresolved-import]

    v, c, st, w = states.values, states.custom_values, states.statics, states.wire_types
    v.bool_value.set(True)
    v.count.set(-123_456)
    v.ratio.set(0.375)
    v.queued_progress.set(0.625)
    v.title.set("ünïcødé 🦀")
    v.optional_value.set(-5)
    v.fixed_numbers.set([1, 65535, 0])
    v.test_enum.set(TestEnum.C)
    v.nested.secondary_choice.set(TestEnum2.X)
    v.nested.selected_enum.set(TestEnum.B)
    c.point.set(TestStruct(-1.5, 2.25, "point"))
    c.optional_struct.set(TestStruct2(True, 7, "seven"))
    st.status_text.set("ready")
    st.summary.set(TestStruct2(False, 65535, "max"))
    st.pair.set([0.5, -0.25])
    st.nested.label.set("label")
    st.nested.enum_hint.set(TestEnum.C)
    w.small_unsigned.set(255)
    w.small_signed.set(-128)
    w.short.set(-32768)
    w.wide.set(2**64 - 1)
    w.single.set(-0.125)
    w.tuple.set((-7, "tuple", True))
    w.list.set([TestStruct2(True, 1, "a"), TestStruct2(False, 2, "b")])
    w.table.set({"pi": 3.5, "neg": -0.5})
    w.keyed_enum.set({TestEnum.A: 1, TestEnum.C: 3})
    w.optional_enum.set(TestEnum2.Z)
    w.bytes.set([0, 127, 255])


def check_client_values(states, signals):
    """Command 31: the values and signals the client sent after command 30."""
    from egui_states_test_bindings.enums import TestEnum, TestEnum2  # ty: ignore[unresolved-import]
    from egui_states_test_bindings.structs import TestStruct  # ty: ignore[unresolved-import]

    v, c, w = states.values, states.custom_values, states.wire_types
    expected = [
        ("v.bool_value", v.bool_value, False),
        ("v.count", v.count, 654_321),
        ("v.ratio", v.ratio, -0.75),
        ("v.queued_progress", v.queued_progress, 0.125),
        ("v.title", v.title, "client ✓"),
        ("v.optional_value", v.optional_value, None),
        ("v.fixed_numbers", v.fixed_numbers, [9, 8, 7]),
        ("v.test_enum", v.test_enum, TestEnum.A),
        ("v.nested.secondary_choice", v.nested.secondary_choice, TestEnum2.Y),
        ("v.nested.selected_enum", v.nested.selected_enum, None),
        ("c.point", c.point, TestStruct(0.5, -0.5, "client")),
        ("c.optional_struct", c.optional_struct, None),
        ("w.small_unsigned", w.small_unsigned, 1),
        ("w.small_signed", w.small_signed, 127),
        ("w.short", w.short, 32767),
        ("w.wide", w.wide, 0),
        ("w.single", w.single, 0.5),
        ("w.tuple", w.tuple, (0, "", False)),
        ("w.list", w.list, []),
        ("w.table", w.table, {}),
        ("w.keyed_enum", w.keyed_enum, {TestEnum.B: 2}),
        ("w.optional_enum", w.optional_enum, None),
    ]
    for name, handle, value in expected:
        actual = handle.get()
        assert actual == value, f"{name}: received {actual!r}, expected {value!r}"
    # Signals run on other workers; give them time to arrive.
    wait_until(lambda: sorted(signals) == ["empty", "enum:C", "number:-2.5", "pair:1.5:B"])


def edit_collections(states):
    """Command 32: server edits of both collections."""
    items = states.value_vec.items
    items.set([5, 6, 7], update=True)
    items.set_item(1, 60, update=True)
    items.add_item(8, update=True)
    items.remove_item(0, update=True)
    mapping = states.value_map.items
    mapping.set({1: 10, 2: 20}, update=True)
    mapping.set_item(3, 30, update=True)
    mapping.set_item(1, 11, update=True)
    mapping.remove_item(2, update=True)


def edit_numeric(states):
    """Command 33: numeric edits and the limits of every element type."""
    d = states.data
    d.bytes.set(np.array([1, 2, 3, 4, 5], dtype=np.uint8))
    d.bytes.add(np.array([6], dtype=np.uint8))
    d.bytes.replace(np.array([20, 30], dtype=np.uint8), 1)
    d.bytes.remove(0, 1)
    d.samples.set(np.array([-1.5, 0.25], dtype=np.float32))
    d.nested.buffer.set(np.array([1, 2, 3], dtype=np.uint16))
    d.nested.buffer.clear()
    for name, dtype in [
        ("u8", np.uint8),
        ("i8", np.int8),
        ("u16", np.uint16),
        ("i16", np.int16),
        ("u32", np.uint32),
        ("i32", np.int32),
        ("u64", np.uint64),
        ("i64", np.int64),
        ("f32", np.float32),
        ("f64", np.float64),
    ]:
        info = np.iinfo(dtype) if np.issubdtype(dtype, np.integer) else np.finfo(dtype)  # ty: ignore[no-matching-overload]
        getattr(states.numeric, f"{name}_data").set(np.array([info.min, 0, info.max], dtype=dtype))
    m = states.multi_data
    m.bytes[1].set(np.array([1, 2], dtype=np.uint8))
    m.bytes[1].add(np.array([3], dtype=np.uint8))
    m.bytes[2].set(np.array([9], dtype=np.uint8))
    m.bytes.remove_index(2)
    m.samples[7].set(np.array([0.5], dtype=np.float32))
    m.nested.buffer[3].set(np.array([4, 5], dtype=np.uint16))
    m.nested.buffer[3].replace(np.array([6], dtype=np.uint16), 1)


def edit_images(states):
    """Command 34: a full image, a partial update, and sparse images."""
    image = states.image.image
    image.set(np.arange(18, dtype=np.uint8).reshape(2, 3, 3))
    image.update(np.array([[[200, 201, 202], [210, 211, 212]]], dtype=np.uint8), origin=(1, 1))
    images = states.image.images
    images[2].set(np.array([[9]], dtype=np.uint8))
    images[7].set_all((2, 2), (1, 2, 3))
    images.remove_index(2)


def configure(states):  # noqa: PLR0915
    """Configure the test protocol; callback results are visible to the probe."""
    st = states.integration
    from egui_states_test_bindings.structs import WirePacket, TestStruct2  # ty: ignore[unresolved-import]
    from egui_states_test_bindings.enums import TestEnum2  # ty: ignore[unresolved-import]

    signals = []
    states.signals.empty_signal.connect(lambda: signals.append("empty"))
    states.signals.number_signal.connect(lambda value: signals.append(f"number:{value}"))
    states.signals.enum_signal.connect(lambda value: signals.append(f"enum:{value.name}"))
    states.wire_types.pair_signal.connect(lambda pair: signals.append(f"pair:{pair[0]}:{pair[1].name}"))

    st.wire.set(
        WirePacket(
            -(2**63),
            2**64 - 1,
            "žluťoučký 🦀",
            -(2**31),
            [0, 32768, 65535],
            TestStruct2(True, 65535, "中"),
            TestEnum2.Z,
        )
    )
    st.wire.connect(lambda packet: st.wire_echo.set(packet, update=True))
    st.value.set(7)
    st.items.set([1, 2])
    st.cached.set(np.array([8, 9], dtype=np.uint8), cache=True)
    st.cached_multi[91].set(np.array([500, 600], dtype=np.uint16), cache=True)
    st.value.connect(lambda value: st.callback_value.set(value, update=True))

    def command(number):  # noqa: PLR0915
        if number == 1:
            st.value.set(21, update=True)
        elif number == 2:
            st.items.add_item(st.value.get(), update=True)
            st.map.set_item(9, 900, update=True)
        elif number == 3:
            states.data_take.take_samples.set(np.array([-1.25, 0, 2.5], dtype=np.float32), update=True)
            states.data_multi_take.samples[513].set(np.array([9.5, -2.25], dtype=np.float32), update=True)
            states.data_multi_take.nested.buffer[24].set(np.array([0, 65535], dtype=np.uint16), update=True)
            st.take.set("payload", update=True)
            st.empty.set(update=True)
            st.data.set(np.array([0, 1, 255], dtype=np.uint8), update=True)
            st.multi[7].set(np.array([100, 200], dtype=np.uint16), update=True)
            st.multi[42].set(np.array([], dtype=np.uint16), update=True)
        elif number == 4:
            st.take.set("", update=True)
            st.data.set(np.array([], dtype=np.uint8), update=True)
        elif number in {10, 11, 12, 13}:

            def send(second):
                if number == 10:
                    st.take.set("" if second else "first", blocking=True, update=True)
                elif number == 11:
                    st.empty.set(blocking=True, update=True)
                elif number == 12:
                    st.data.set(np.array([] if second else [1, 2], dtype=np.uint8), blocking=True, update=True)
                else:
                    st.multi[73].set(
                        np.array([] if second else [100, 200], dtype=np.uint16), blocking=True, update=True
                    )

            send(False)
            st.phase.set(1, update=True)
            send(True)
            st.phase.set(2, update=True)
        elif number == 20:
            st.cached.set(np.array([31, 32], dtype=np.uint8), cache=True, update=True)
            st.cached_multi.remove_index(91, update=True)
            st.cached_multi[513].set(np.array([17], dtype=np.uint16), cache=True, update=True)
            st.phase.set(20, update=True)
        elif number == 21:
            st.cached.set(np.array([41], dtype=np.uint8), cache=False, update=True)
            st.cached_multi.reset(update=True)
            st.phase.set(21, update=True)
        elif number == 22:
            st.multi[7].set(np.array([7], dtype=np.uint16), update=True)
            st.multi[900].set(np.array([9], dtype=np.uint16), update=True)
            st.multi.remove_index(7, update=True)
            st.phase.set(22, update=True)
        elif number == 23:
            st.multi.reset(update=True)
            st.phase.set(23, update=True)
        elif number in (24, 25):  # noqa: PLR6201
            st.multi[7].set(np.array([7], dtype=np.uint16), blocking=True, update=True)
            if number == 24:
                st.multi.remove_index(7, update=True)
            else:
                st.multi.reset(update=True)
            st.phase.set(24, update=True)
            st.multi[513].set(np.array([9], dtype=np.uint16), blocking=True, update=True)
            st.phase.set(25, update=True)
        elif number == 30:
            set_wire_values(states)
            st.phase.set(30, update=True)
        elif number == 31:
            try:
                check_client_values(states, signals)
            finally:
                st.phase.set(31, update=True)
        elif number in {32, 33, 34}:
            {32: edit_collections, 33: edit_numeric, 34: edit_images}[number](states)
            st.phase.set(number, update=True)
        elif number == 99:
            st.phase.set(99, update=True)
        else:
            raise AssertionError(f"unknown command: {number}")

    st.command.connect(command)


def run_scenario(probe, scenario):
    """Run the server and its potentially blocking callbacks in a disposable process."""
    errors, warnings = [], []
    server = StatesServer(error_handler=errors.append, version=StatesServer.VERSION_HASH)
    from egui_states.logging import LogLevel

    server.logging.add_logger(LogLevel.Error, lambda message: errors.append(RuntimeError(message)))
    server.logging.add_logger(LogLevel.Warning, warnings.append)
    configure(server.states)
    port = free_port()
    try:
        server.start(port, (127, 0, 0, 1))
        run([probe, port, scenario], timeout=30)

    finally:
        server.stop()
        assert not errors, f"Unexpected callback errors: {errors!r}"
    if scenario == "rejected-client":
        wait_until(lambda: any("wrong version" in warning for warning in warnings))
