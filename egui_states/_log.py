"""Forward the server's diagnostics to Python's ``logging``."""

import logging

from egui_states.signals import SignalsManager

_LOGGING_ID = 0

#: The logger that receives the server's diagnostics.
logger = logging.getLogger("egui_states")

# The native server's level ids, in its order: debug, info, warning, error.
_LEVELS = (logging.DEBUG, logging.INFO, logging.WARNING, logging.ERROR)


def _forward(message: tuple[int, str]) -> None:
    level, text = message
    logger.log(_LEVELS[level] if 0 <= level < len(_LEVELS) else logging.WARNING, text)


def forward_logs(signals_manager: SignalsManager) -> None:
    """Send the server's diagnostics to the ``egui_states`` logger.

    They run on the callback workers, like any other callback, so a slow
    handler never holds up the connection.

    Args:
        signals_manager (SignalsManager): Callback dispatcher of the server.
    """
    signals_manager.add_callback(_LOGGING_ID, _forward)
