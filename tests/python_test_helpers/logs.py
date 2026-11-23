"""Collect what the server logs to the ``egui_states`` logger."""

from collections.abc import Generator
from contextlib import contextmanager
import logging


class _ServerLogs(logging.Handler):
    def __init__(self, errors: list, warnings: list[str] | None) -> None:
        super().__init__(logging.WARNING)
        self._errors = errors
        self._warnings = warnings

    def emit(self, record: logging.LogRecord) -> None:
        if record.levelno >= logging.ERROR:
            self._errors.append(RuntimeError(record.getMessage()))
        elif self._warnings is not None:
            self._warnings.append(record.getMessage())


@contextmanager
def server_logs(errors: list, warnings: list[str] | None = None) -> Generator[None]:
    """Append the server's errors, as ``RuntimeError``, and its warnings.

    Args:
        errors (list): Receives every error, next to callback errors.
        warnings (list[str] | None, optional): Receives every warning, or
            ``None`` to ignore them.

    Yields:
        None: While the messages are collected.
    """
    handler = _ServerLogs(errors, warnings)
    logger = logging.getLogger("egui_states")
    logger.addHandler(handler)
    try:
        yield
    finally:
        logger.removeHandler(handler)
