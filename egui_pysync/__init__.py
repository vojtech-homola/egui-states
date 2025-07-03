"""TODO: Doc string."""

from egui_pysync import structures, version
from egui_pysync.server import StateServer
from egui_pysync.structures import (
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
from egui_pysync.version import __version__

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
