import threading
import time

import numpy as np
import cv2
import matplotlib.pyplot as plt

from egui_pysync import enums, types
from egui_pysync.enums import OpticalMode, View
from egui_pysync.server import StateServer

state_server = StateServer(signals_workers=10)
state_server.start()


def error_callback(error: str):
    print(error)


state_server.error.connect(error_callback)


def callback(mode: OpticalMode):
    # print("thread id: ", threading.get_ident())
    # time.sleep(1)
    print(mode.name)


states = state_server.states
states.optics.optical_mode.connect(callback)


def set_periodic_table(change: types.Element):
    if change[1]:
        if len(states.xray.elements.get()) < 14:
            states.xray.elements.set_item(change[0], True, update=True)
    else:
        states.xray.elements.remove_item(change[0], update=True)


def add_periodic_table(text: str):
    print(text)


states.xray.setter.connect(set_periodic_table)
states.xray.add_element.connect(add_periodic_table)
