"""Feature showcase with a Python server. Importing this module starts nothing."""

import argparse
from collections.abc import Callable
import signal
import threading

import numpy as np
from showcase_bindings import ShowcaseState, StatesServer
from showcase_bindings.enums import PrimaryChoice, SecondaryChoice
from showcase_bindings.structs import Point, Summary
from egui_states import LogLevel

PORT = 8091
DEFAULT_VEC = [10, -3, 27]
DEFAULT_MAP = {1: 100, 2: 200, 5: 500}


# Keep the showcase's callbacks together so the example is easy to follow.
def register_callbacks(server: StatesServer) -> None:  # noqa: PLR0915
    """Connect logging and interactive actions to a server instance.

    Args:
        server (StatesServer): Server whose state handles receive callbacks.
    """
    states = server.states

    def _print_debug(message: str) -> None:
        print("Debug:", message)

    def _print_info(message: str) -> None:
        print("Info:", message)

    def _print_warning(message: str) -> None:
        print("Warning:", message)

    def _print_error(message: str) -> None:
        print("Error:", message)

    def on_ratio(value: float) -> None:
        print(f"ratio changed: {value:.3f}")

    def on_title(value: str) -> None:
        print(f"title changed: {value}")

    def on_count(value: int, previous: int) -> None:
        print(f"count changed: {previous} -> {value}")

    def on_enum(value: PrimaryChoice) -> None:
        print(f"enum changed: {value.name}")

    def on_empty_signal() -> None:
        print("empty signal emitted")

    def on_number_signal(value: float) -> None:
        print(f"number signal emitted: {value:.3f}")

    def on_enum_signal(value: PrimaryChoice) -> None:
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

    server.logging.add_logger(LogLevel.Debug, _print_debug)
    server.logging.add_logger(LogLevel.Info, _print_info)
    server.logging.add_logger(LogLevel.Warning, _print_warning)
    server.logging.add_logger(LogLevel.Error, _print_error)

    states.values.ratio.connect(on_ratio)
    states.values.title.connect(on_title)
    states.values.count.connect_previous(on_count)
    states.values.primary_choice.connect(on_enum)
    states.signals.empty_signal.connect(on_empty_signal)
    states.signals.number_signal.connect(on_number_signal)
    states.signals.enum_signal.connect(on_enum_signal)

    states.value_vec.actions.append_item.connect(_append_value_vec)
    states.value_vec.actions.remove_last.connect(_remove_last_value_vec)
    states.value_vec.actions.reset_demo.connect(_reset_value_vec)

    states.value_map.actions.insert_next.connect(_insert_next_value_map)
    states.value_map.actions.remove_lowest.connect(_remove_lowest_value_map)
    states.value_map.actions.reset_demo.connect(_reset_value_map)


# Keep each state kind's initial values in one visible setup sequence.
def populate_initial_data(states: ShowcaseState) -> None:  # noqa: PLR0915
    """Populate the same deterministic data as the Rust showcase.

    Args:
        states (ShowcaseState): Generated server state handles to populate.
    """
    states.values.bool_value.set(True)
    states.values.count.set(7)
    states.values.ratio.set(0.42, set_signal=True)
    states.values.queued_progress.set(0.25)
    states.values.title.set("Interactive egui-states showcase", set_signal=True)
    states.values.optional_value.set(12)
    states.values.fixed_numbers.set([2, 4, 8])
    states.values.primary_choice.set(PrimaryChoice.C, set_signal=True)
    states.values.nested.secondary_choice.set(SecondaryChoice.Z)
    states.values.nested.selected_enum.set(PrimaryChoice.B)

    states.statics.status_text.set("Static values are shown as labels.")
    states.statics.summary.set(Summary(True, 3, "static summary"))
    states.statics.pair.set([0.5, 1.5])
    states.statics.nested.label.set("Nested static label")
    states.statics.nested.enum_hint.set(PrimaryChoice.A)

    states.custom_values.point.set(Point(1.5, -0.75, "editable point"))
    states.custom_values.optional_struct.set(Summary(True, 9, "optional payload"))

    states.value_vec.items.set(list(DEFAULT_VEC), update=True)
    states.value_map.items.set(dict(DEFAULT_MAP), update=True)

    states.data.bytes.set(np.arange(32, dtype=np.uint8), update=True)
    states.data.samples.set(np.linspace(0.0, 1.0, 1024 * 20, dtype=np.float32), update=True)
    states.data.nested.buffer.set(np.arange(8, dtype=np.uint16), update=True)

    states.multi_data.bytes[0].set(np.arange(12, dtype=np.uint8), update=True)
    states.multi_data.bytes[0].replace(np.array([200, 201], dtype=np.uint8), 4, update=True)
    states.multi_data.bytes[1].set(np.array([10, 20, 30], dtype=np.uint8), update=True)
    states.multi_data.bytes[1].add(np.array([40, 50], dtype=np.uint8), update=True)

    states.multi_data.samples[0].set(np.linspace(0.0, 1.0, 8, dtype=np.float32), update=True)
    states.multi_data.samples[2].set(np.linspace(-1.0, 1.0, 5, dtype=np.float32), update=True)
    states.multi_data.samples[2].add(np.array([1.5, 2.0], dtype=np.float32), update=True)

    states.multi_data.nested.buffer[0].set(np.arange(4, dtype=np.uint16), update=True)
    states.multi_data.nested.buffer[0].add(np.array([10, 11], dtype=np.uint16), update=True)
    states.multi_data.nested.buffer[3].set(np.array([100, 110, 120], dtype=np.uint16), update=True)

    states.value_take.take_text.set("ValueTake payload from server", update=True)
    states.value_take.take_empty.set(update=True)

    states.data_take.take_buffer.set(np.arange(16, dtype=np.uint8), update=True, cache=True)
    states.data_take.take_samples.set(np.linspace(0.0, 1.0, 8, dtype=np.float32), update=True, cache=True)

    states.data_multi_take.bytes[0].set(np.arange(6, dtype=np.uint8), update=True, cache=True)
    states.data_multi_take.bytes[1].set(np.array([10, 20, 30], dtype=np.uint8), update=True, cache=True)

    states.data_multi_take.samples[0].set(np.linspace(0.0, 1.0, 4, dtype=np.float32), update=True, cache=True)
    states.data_multi_take.samples[2].set(np.linspace(-1.0, 1.0, 3, dtype=np.float32), update=True, cache=True)

    states.data_multi_take.nested.buffer[0].set(np.arange(4, dtype=np.uint16), update=True, cache=True)
    states.data_multi_take.nested.buffer[3].set(np.array([100, 110], dtype=np.uint16), update=True, cache=True)

    y, x = np.indices((256, 256), dtype=np.uint16)
    image = np.stack((x, y, (x + y) // 2), axis=-1).astype(np.uint8)
    states.image.image.set(image, update=True)
    image_patch = np.zeros((64, 64, 4), dtype=np.uint8)
    image_patch[..., 1] = 220
    image_patch[..., 3] = 255
    states.image.image.update(image_patch, origin=(96, 96), update=True)

    states.image.images[2].set(image, update=True)
    states.image.images[7].set_all((256, 256), (25, 35, 75), update=True)
    states.image.images[7].update(image_patch, origin=(96, 96), update=True)


def setup_server(*, error_handler: Callable[[Exception], None] | None = None) -> StatesServer:
    """Create and configure a server without opening a listening socket.

    Args:
        error_handler (Callable[[Exception], None] | None, optional): Handler
            for asynchronous callback errors; None uses the default handler.

    Returns:
        StatesServer: Configured server; call start to listen for a client.
    """
    server = StatesServer(error_handler=error_handler)
    register_callbacks(server)
    populate_initial_data(server.states)
    return server


def main() -> None:
    """Run until interrupted and release the listening socket on exit."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=PORT)
    parser.add_argument("--web", help="directory of a built browser GUI to serve on the port")
    args = parser.parse_args()
    stopped = threading.Event()
    signal.signal(signal.SIGINT, lambda *_: stopped.set())
    signal.signal(signal.SIGTERM, lambda *_: stopped.set())
    server = setup_server()
    if args.web is not None:
        server.serve_web(args.web)
    try:
        server.start(args.port, (127, 0, 0, 1))
        print(f"Showcase server listening on {args.port}", flush=True)
        # Another thread may take the signal, which only flags it for the
        # main thread; a timed wait lets the main thread run the handler.
        while not stopped.wait(0.5):
            pass
    finally:
        server.stop()


if __name__ == "__main__":
    main()
