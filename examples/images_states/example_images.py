import numpy as np
from images import core
from images.states import States

from egui_pysync import StateServer

state_server = StateServer(States, core, port=8081)
state_server.start()
states = state_server.states

states.text.set("Hello, world!")
image = (np.random.default_rng(0).random((256, 256)) * 255).astype(np.uint8)
states.image.set(image, update=True)

chunk = np.zeros((256,), dtype=np.uint8)
start = 256
states.image.set_chunk(chunk, start, update=True)
