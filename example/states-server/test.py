# ruff: noqa: D103
import numpy as np
from egui_states import LogLevel

from states import StatesServer

state_server = StatesServer(port=8081)
state_server.start()
states = state_server.states

image = (np.random.default_rng(0).random((256, 256)) * 255).astype(np.uint8)
# image = np.ones((256, 256), dtype=np.uint8) * 60
states.image.set(image, update=True)

graph = np.ones((100,), dtype=np.float32)
states.graphs.set(graph, 0)


def print_debug(d: str):
    print("Debug:", d)


def print_info(i: str):
    print("Info:", i)


def print_warning(w: str):
    print("Warning:", w)


def print_error(e: str):
    print("Error:", e)


state_server.logging.add_logger(LogLevel.Debug, print_debug)
state_server.logging.add_logger(LogLevel.Info, print_info)
state_server.logging.add_logger(LogLevel.Warning, print_warning)
state_server.logging.add_logger(LogLevel.Error, print_error)


def on_value(value: float):
    print("Value changed:", value)


states.value.connect(on_value)


def on_empty_signal():
    print("Empty signal emitted")


states.empty_signal.connect(on_empty_signal)
