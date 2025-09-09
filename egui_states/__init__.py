"""TODO: Doc string."""

from egui_states import structures, version
from egui_states.server import StateServer
from egui_states.structures import (
    Graph,
    Signal,
    SignalEmpty,
    Value,
    ValueDict,
    ValueGraphs,
    ValueImage,
    ValueList,
    ValueStatic,
)
from egui_states.version import __version__

__all__ = [
    "version",
    "__version__",
    "StateServer",
    "structures",
    "Signal",
    "SignalEmpty",
    "Value",
    "ValueDict",
    "ValueGraphs",
    "ValueImage",
    "ValueList",
    "ValueStatic",
    "Graph",
]
