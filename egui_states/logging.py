"""Dispatch log messages emitted by the connected egui client."""

from collections.abc import Callable
from enum import Enum

from egui_states.signals import SignalsManager

_LOGGING_ID = 0


class LogLevel(Enum):
    """Severity of a diagnostic message emitted by the egui client."""

    #: Diagnostic information useful while debugging.
    Debug = 0
    #: Routine informational output.
    Info = 1
    #: A potentially problematic condition.
    Warning = 2
    #: An error reported by the client.
    Error = 3


class LoggingSignal:
    """Dispatch log messages emitted by the connected egui client."""

    def __init__(self, signals_manager: SignalsManager) -> None:
        """Initialize the logging signal.

        Args:
            signals_manager (SignalsManager): Callback dispatcher to register
                with.
        """
        self._loggers: dict[int, list[Callable[[str], None]]] = {0: [], 1: [], 2: [], 3: []}
        signals_manager.add_callback(_LOGGING_ID, self._callback)

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

    def add_logger(self, level: LogLevel, logger: Callable[[str], None]) -> None:
        """Add logger for a specific level.

        Args:
            level (LogLevel): Logging level to receive.
            logger (Callable[[str], None]): Callback that receives each client
                message at exactly this level.
        """
        self._loggers[level.value].append(logger)

    def remove_logger(self, level: LogLevel, logger: Callable[[str], None]) -> None:
        """Remove logger for a specific level.

        Args:
            level (LogLevel): Logging level from which to remove the callback.
            logger (Callable[[str], None]): Previously registered callback.
        """
        if logger in self._loggers[level.value]:
            self._loggers[level.value].remove(logger)

    def remove_all_loggers(self, level: LogLevel) -> None:
        """Remove all loggers for a specific level.

        Args:
            level (LogLevel): Logging level whose callbacks should be removed.
        """
        self._loggers[level.value].clear()
