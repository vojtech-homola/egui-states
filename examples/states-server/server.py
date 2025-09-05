import numpy as np
from states_server import core
from states_server.states import States

from egui_pysync import StateServer

state_server = StateServer(States, core, port=8081)
state_server.start()
states = state_server.states

# image = (np.random.default_rng(0).random((256, 256)) * 255).astype(np.uint8)
# image = np.ones((256, 256), dtype=np.uint8) * 60
# states.image.set(image, update=True)


def print_error(e: str):
    print("Error:", e)


state_server.error.connect(print_error)


def on_value(value: float):
    print("Value changed:", value)


states.value.connect(on_value)
