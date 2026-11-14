# ruff: noqa: D103
"""File transfer endpoints without a client: refused sends and the public types."""

import pickle

import pytest

from egui_states import TransferError, TransferErrorKind, TransferProgress, TransferStatus
from egui_states_test_bindings import (  # ty: ignore[import-not-found]
    State,
    StatesServer,
)
from egui_states_test_bindings.structs import TransferInfo  # ty: ignore[import-not-found]


def test_sends_without_a_client_are_refused(
    local_bundle: tuple[StatesServer, State, list[Exception]], tmp_path
) -> None:
    _server, states, _errors = local_bundle
    t = states.transfers
    with pytest.raises(TransferError) as error:
        t.files.send_bytes(b"data", "a.bin")
    assert error.value.kind is TransferErrorKind.NotConnected
    with pytest.raises(TransferError) as error:
        t.results.send_bytes(b"data", "a.bin", meta=TransferInfo(1, "x"))
    assert error.value.kind is TransferErrorKind.NotConnected

    path = tmp_path / "a.bin"
    path.write_bytes(b"data")
    with pytest.raises(TransferError) as error:
        t.files.send_file(path)
    assert error.value.kind is TransferErrorKind.NotConnected
    with pytest.raises(TransferError):
        t.files.send_files([path], "set", root=tmp_path)
    assert t.files.last() is None


def test_invalid_sends_raise_builtin_errors(
    local_bundle: tuple[StatesServer, State, list[Exception]], tmp_path
) -> None:
    _server, states, _errors = local_bundle
    t = states.transfers
    with pytest.raises(OSError, match="missing"):
        t.files.send_file(tmp_path / "missing")
    outside = tmp_path / "outside.bin"
    outside.write_bytes(b"x")
    with pytest.raises(ValueError, match="not inside"):
        t.files.send_files([outside], "set", root=tmp_path / "root")
    with pytest.raises(ValueError, match="timeout needs wait"):
        t.files.send_bytes(b"x", "x.bin", timeout=1.0)
    with pytest.raises(ValueError, match="finite"):
        t.files.send_bytes(b"x", "x.bin", wait=True, timeout=-1.0)
    with pytest.raises(TypeError):
        t.results.send_bytes(b"x", "x.bin")  # meta is required


def test_public_types() -> None:
    error = TransferError("rejected: nope", 2)
    assert error.kind is TransferErrorKind.Rejected
    assert str(error) == "rejected: nope"
    assert isinstance(error, Exception)
    progress = TransferProgress(5, 10, 0, 1)
    assert progress.total == 10
    assert pickle.loads(pickle.dumps(TransferStatus.Done)) is TransferStatus.Done


def test_callbacks_connect_and_disconnect(
    local_bundle: tuple[StatesServer, State, list[Exception]],
) -> None:
    _server, states, _errors = local_bundle
    files = states.transfers.files
    calls = []
    files.connect(calls.append)
    files.connect(print)
    files.disconnect(calls.append)
    files.disconnect(calls.append)
    files.disconnect_all()
    assert not files._dispatching
