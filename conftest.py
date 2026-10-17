"""Prepare the current native extension before collecting either Python suite."""

import sys
import pytest

from tests.preparation import prepare_extension


@pytest.hookimpl(tryfirst=True)
def pytest_configure(config):
    """Build once per pytest process, before importing test modules."""
    try:
        config._test_extension = prepare_extension()
    except (RuntimeError, OSError, KeyError) as error:
        raise pytest.UsageError(f"Native test preparation failed: {error}") from error
    print(f"Test interpreter: {sys.executable}\nFresh extension: {config._test_extension}", flush=True)


def pytest_report_header(config):
    return f"Fresh extension: {config._test_extension}"
