"""Exact log levels and callback removal through the existing log dispatcher."""

from egui_states.logging import LogLevel


def test_loggers_filter_exact_level_and_can_be_removed(local_bundle):
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
