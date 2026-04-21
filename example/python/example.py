# ruff: noqa: D103
import numpy as np
from states_server import StatesServer, TestEnum, TestEnum2, TestStruct, TestStruct2

from egui_states import LogLevel

PORT = 8091
server = StatesServer(port=PORT)
server.start()
states = server.states

DEFAULT_VEC = [10, -3, 27]
DEFAULT_MAP = {1: 100, 2: 200, 5: 500}


def _print_debug(message: str) -> None:
    print("Debug:", message)


def _print_info(message: str) -> None:
    print("Info:", message)


def _print_warning(message: str) -> None:
    print("Warning:", message)


def _print_error(message: str) -> None:
    print("Error:", message)


server.logging.add_logger(LogLevel.Debug, _print_debug)
server.logging.add_logger(LogLevel.Info, _print_info)
server.logging.add_logger(LogLevel.Warning, _print_warning)
server.logging.add_logger(LogLevel.Error, _print_error)


def on_ratio(value: float) -> None:
    print(f"ratio changed: {value:.3f}")


def on_title(value: str) -> None:
    print(f"title changed: {value}")


def on_enum(value: TestEnum) -> None:
    print(f"enum changed: {value.name}")


def on_empty_signal() -> None:
    print("empty signal emitted")


def on_number_signal(value: float) -> None:
    print(f"number signal emitted: {value:.3f}")


def on_enum_signal(value: TestEnum) -> None:
    print(f"enum signal emitted: {value.name}")


def _reset_value_vec() -> None:
    states.value_vec.items.set(list(DEFAULT_VEC), update=True)


def _append_value_vec() -> None:
    current = states.value_vec.items.get()
    next_value = current[-1] + 5 if current else DEFAULT_VEC[0]
    states.value_vec.items.add_item(next_value, update=True)
    print(f"value_vec appended: {next_value}")


def _remove_last_value_vec() -> None:
    current = states.value_vec.items.get()
    if current:
        states.value_vec.items.remove_item(len(current) - 1, update=True)
        print("value_vec removed last item")


def _reset_value_map() -> None:
    states.value_map.items.set(dict(DEFAULT_MAP), update=True)


def _insert_next_value_map() -> None:
    current = states.value_map.items.get()
    next_key = max(current, default=0) + 1
    states.value_map.items.set_item(next_key, next_key * 100, update=True)
    print(f"value_map inserted: {next_key} -> {next_key * 100}")


def _remove_lowest_value_map() -> None:
    current = states.value_map.items.get()
    if current:
        lowest_key = min(current)
        states.value_map.items.remove_item(lowest_key, update=True)
        print(f"value_map removed key: {lowest_key}")


states.values.ratio.connect(on_ratio)
states.values.title.connect(on_title)
states.values.test_enum.connect(on_enum)
states.signals.empty_signal.connect(on_empty_signal)
states.signals.number_signal.connect(on_number_signal)
states.signals.enum_signal.connect(on_enum_signal)

states.value_vec.actions.append_item.connect(_append_value_vec)
states.value_vec.actions.remove_last.connect(_remove_last_value_vec)
states.value_vec.actions.reset_demo.connect(_reset_value_vec)

states.value_map.actions.insert_next.connect(_insert_next_value_map)
states.value_map.actions.remove_lowest.connect(_remove_lowest_value_map)
states.value_map.actions.reset_demo.connect(_reset_value_map)

states.values.bool_value.set(True)
states.values.count.set(7)
states.values.ratio.set(0.42, set_signal=True)
states.values.queued_progress.set(0.25)
states.values.title.set("Interactive egui-states example", set_signal=True)
states.values.optional_value.set(12)
states.values.fixed_numbers.set([2, 4, 8])
states.values.test_enum.set(TestEnum.C, set_signal=True)
states.values.nested.secondary_choice.set(TestEnum2.Z)
states.values.nested.selected_enum.set(TestEnum.B)

states.statics.status_text.set("Static values are shown as labels.")
states.statics.summary.set(TestStruct2(True, 3, "static summary"))
states.statics.pair.set([0.5, 1.5])
states.statics.nested.label.set("Nested static label")
states.statics.nested.enum_hint.set(TestEnum.A)

states.custom_values.point.set(TestStruct(1.5, -0.75, "editable point"))
states.custom_values.optional_struct.set(TestStruct2(True, 9, "optional payload"))

states.data.bytes.set(np.arange(32, dtype=np.uint8), update=True)
states.data.samples.set(np.linspace(0.0, 1.0, 12, dtype=np.float32), update=True)
states.data.nested.buffer.set(np.arange(8, dtype=np.uint16), update=True)

states.value_take.take_text.set("ValueTake payload from Python", update=True)
states.value_take.take_empty.set(update=True)

rng = np.random.default_rng()
image = (rng.random((256, 256, 3)) * 255).astype(np.uint8)
states.image.image.set(image, update=True)
