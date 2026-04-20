# ruff: noqa: D103
import numpy as np
from states_server import StatesServer, TestEnum, TestEnum2, TestStruct, TestStruct2

from egui_states import LogLevel

PORT = 8091
server = StatesServer(port=PORT)
server.start()
states = server.states
callback_log: list[str] = []


def _record(message: str) -> None:
    callback_log.append(message)
    print(message)


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
    _record(f"ratio changed: {value:.3f}")


def on_title(value: str) -> None:
    _record(f"title changed: {value}")


def on_enum(value: TestEnum) -> None:
    _record(f"enum changed: {value.name}")


def on_empty_signal() -> None:
    _record("empty signal emitted")


def on_number_signal(value: float) -> None:
    _record(f"number signal emitted: {value:.3f}")


def on_enum_signal(value: TestEnum) -> None:
    _record(f"enum signal emitted: {value.name}")


states.scalars.ratio.connect(on_ratio)
states.scalars.title.connect(on_title)
states.scalars.test_enum.connect(on_enum)
states.events.empty_signal.connect(on_empty_signal)
states.events.number_signal.connect(on_number_signal)
states.events.enum_signal.connect(on_enum_signal)

states.scalars.bool_value.set(True)
states.scalars.count.set(7)
states.scalars.ratio.set(0.42, set_signal=True)
states.scalars.queued_progress.set(0.25)
states.scalars.title.set("Interactive egui-states example", set_signal=True)
states.scalars.optional_value.set(12)
states.scalars.fixed_numbers.set([2, 4, 8])
states.scalars.test_enum.set(TestEnum.C, set_signal=True)

states.statics.status_text.set("Static values are shown as labels.")
states.statics.summary.set(TestStruct2(True, 3, "static summary"))
states.statics.pair.set([0.5, 1.5])

states.custom.point.set(TestStruct(1.5, -0.75, "editable point"))
states.custom.choice.set(TestEnum2.Z)
states.custom.optional_struct.set(TestStruct2(True, 9, "optional payload"))

states.collections.plain_vec_value.set([4, 8, 15, 16, 23, 42])
states.collections.list.set([10, -3, 27])
states.collections.map.set({1: 100, 2: 200, 5: 500})

states.nested.label.set("Nested substate")
states.nested.counter.set(3)
states.nested.inner.selected.set(TestEnum.B)
states.nested.inner.pair.set([9.0, 12.0])
states.nested.inner.leaf.enabled.set(True)
states.nested.inner.leaf.message.set("Leaf text value")

image_y = np.linspace(0, 255, 128, dtype=np.uint8)
image_x = np.linspace(255, 0, 128, dtype=np.uint8)
image = np.zeros((128, 128, 4), dtype=np.uint8)
image[..., 0] = image_y[:, None]
image[..., 1] = image_x[None, :]
image[..., 2] = 96
image[..., 3] = 255
states.data.image.set(image, update=True)

states.data.bytes.set(np.arange(32, dtype=np.uint8), update=True)
states.data.samples.set(np.linspace(0.0, 1.0, 12, dtype=np.float32), update=True)
states.nested.inner.leaf.buffer.set(np.arange(8, dtype=np.uint16), update=True)

states.events.take_text.set("ValueTake payload from Python", update=True)
states.events.take_empty.set(update=True)


def emit_demo_events() -> None:
    states.events.empty_signal.set()
    states.events.number_signal.set(states.scalars.ratio.get())
    states.events.enum_signal.set(states.scalars.test_enum.get())


def stop_server() -> None:
    if server.is_running():
        server.stop()
