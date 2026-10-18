"""Exact log levels and callback removal in the Python log dispatcher.

Native log messages reaching these loggers are checked by the rejected-client
integration scenario.
"""

from egui_states.logging import LogLevel


def test_logger_dispatch_filters_levels_and_removes_loggers(local_bundle):
    """Dispatch each level only to registered loggers and honor their removal."""
    server, _, _ = local_bundle
    info, warning = [], []
    server.logging.add_logger(LogLevel.Info, info.append)
    server.logging.add_logger(LogLevel.Warning, warning.append)
    for level, message in [(LogLevel.Debug, "debug"), (LogLevel.Info, "info"), (LogLevel.Warning, "warning")]:
        server.logging._callback((level.value, message))
    assert info == ["info"]
    assert warning == ["warning"]
    server.logging.remove_logger(LogLevel.Info, info.append)
    server.logging._callback((LogLevel.Info.value, "removed"))
    server.logging.remove_all_loggers(LogLevel.Warning)
    server.logging._callback((LogLevel.Warning.value, "cleared"))
    assert info == ["info"]
    assert warning == ["warning"]
