from collections.abc import Callable
from enum import Enum

from egui_states._core import StateServerCore
from egui_states.signals import SignalsManager


class LogLevel(Enum):
    """Logging levels."""

    Debug = 0
    Info = 1
    Warning = 2
    Error = 3


class LoggingSignal:
    """Logging signal for processing log messages from the state server."""

    def __init__(self, signals_manager: SignalsManager, server: StateServerCore) -> None:
        """Initialize the LoggingSignal."""
        self._loggers: dict[int, list[Callable[[str], None]]] = {0: [], 1: [], 2: [], 3: []}
        logging_id = server.signal_get_logging_id()
        signals_manager.add_callback(logging_id, self._callback)
        server.signal_set_to_multi(logging_id)

    def _callback(self, message: tuple[int, str]) -> None:
        level = message[0]
        if level == LogLevel.Debug.value:
            for logger in self._loggers[0]:
                logger(message[1])
        elif level == LogLevel.Info.value:
            for logger in self._loggers[1]:
                logger(message[1])
        elif level == LogLevel.Warning.value:
            for logger in self._loggers[2]:
                logger(message[1])
        elif level == LogLevel.Error.value:
            for logger in self._loggers[3]:
                logger(message[1])

    def add_logger(self, level: LogLevel, logger: Callable[[str], None]):
        """Add logger for a specific level.

        Args:
            level(Level): The logging level.
            logger(Callable[[str], None]): The logger to add.
        """
        self._loggers[level.value].append(logger)

    def remove_logger(self, level: LogLevel, logger: Callable[[str], None]):
        """Remove logger for a specific level.

        Args:
            level(Level): The logging level.
            logger(Callable[[str], None]): The logger to remove.
        """
        if logger in self._loggers[level.value]:
            self._loggers[level.value].remove(logger)

    def remove_all_loggers(self, level: LogLevel):
        """Remove all loggers for a specific level.

        Args:
            level(Level): The logging level.
        """
        self._loggers[level.value].clear()
