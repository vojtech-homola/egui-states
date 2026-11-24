"""The server's diagnostics in Python's logging.

That a native warning reaches the logger is checked by the rejected-client
integration scenario.
"""

import logging

from egui_states import _log  # noqa: PLC2701 - the forwarder has no public name


def test_server_messages_reach_the_egui_states_logger_at_their_level(caplog):
    """Map each native level to its logging level on the ``egui_states`` logger."""
    caplog.set_level(logging.DEBUG, logger="egui_states")
    for level, text in enumerate(["debug", "info", "warning", "error"]):
        _log._forward((level, text))
    _log._forward((9, "unknown level"))
    records = [(r.name, r.levelno, r.getMessage()) for r in caplog.records]
    assert records == [
        ("egui_states", logging.DEBUG, "debug"),
        ("egui_states", logging.INFO, "info"),
        ("egui_states", logging.WARNING, "warning"),
        ("egui_states", logging.ERROR, "error"),
        ("egui_states", logging.WARNING, "unknown level"),
    ]
