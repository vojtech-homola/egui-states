# ruff: noqa: D103
import numpy as np
from egui_states import LogLevel

from states_server import StatesServer, TestEnum

server = StatesServer(port=8091)
server.start()
states = server.states

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


server.logging.add_logger(LogLevel.Debug, print_debug)
server.logging.add_logger(LogLevel.Info, print_info)
server.logging.add_logger(LogLevel.Warning, print_warning)
server.logging.add_logger(LogLevel.Error, print_error)


def on_value(value: float):
    print("Value changed:", value)


states.value.connect(on_value)


def on_empty_signal():
    print("Empty signal emitted")


states.empty_signal.connect(on_empty_signal)


# def on_value2(value: float):
#     print("Value2 changed:", value)


# states.value2.connect(on_value2)


def on_test_enum(value: TestEnum):
    print("Test enum changed:", type(value))


states.test_enum.connect(on_test_enum)
