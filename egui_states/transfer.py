# ruff: noqa: D105 D107
"""Handles and errors of the file transfer endpoints."""

from __future__ import annotations

from enum import Enum
from pathlib import Path
from typing import Any, NamedTuple

from os import PathLike, fspath

from egui_states._core import _TransferHandle, _TransferOffer


class TransferStatus(Enum):
    """Where a transfer stands."""

    #: Waiting for the other side's answer, or behind a transfer of another
    #: endpoint.
    Pending = 0
    #: Data is flowing.
    Running = 1
    #: The receiver has saved everything.
    Done = 2
    #: The transfer ended without success; see :meth:`TransferHandle.error`.
    Failed = 3


class TransferErrorKind(Enum):
    """Why a transfer failed or a send was refused."""

    #: The endpoint already has an unfinished transfer.
    Busy = 0
    #: No client is connected.
    NotConnected = 1
    #: The receiver rejected the offer; the message is its reason.
    Rejected = 2
    #: A side cancelled the transfer.
    Cancelled = 3
    #: The client went away or the server stopped.
    Disconnected = 4
    #: Reading or writing a file failed.
    Io = 5
    #: The offer cannot be accepted that way, or a file name cannot be used.
    Unsupported = 6
    #: The other side broke the transfer protocol.
    Protocol = 7
    #: The data exceeds the limit of an in-memory accept.
    TooLarge = 8


class TransferError(Exception):
    """A file transfer failed or a send was refused.

    Args:
        message (str): What happened.
        kind (TransferErrorKind | int): The kind of failure.
    """

    def __init__(self, message: str, kind: TransferErrorKind | int) -> None:
        super().__init__(message)
        self.kind: TransferErrorKind = TransferErrorKind(kind)


class TransferProgress(NamedTuple):
    """How far a transfer has come."""

    #: Bytes across all files.
    done: int
    #: Bytes in total; always known in this version.
    total: int | None
    #: Files completely transferred.
    files_done: int
    #: Files in the transfer.
    files_total: int


class TransferHandle:
    """One upload or download.

    A handle can be used from any thread. Dropping it does not cancel the
    transfer.
    """

    __slots__ = ("_inner",)

    def __init__(self, inner: _TransferHandle) -> None:
        self._inner = inner

    def name(self) -> str:
        """Return the file name, or the name of the set.

        Returns:
            str: The name the receiver sees.
        """
        return self._inner.name()

    def status(self) -> TransferStatus:
        """Return where the transfer stands.

        Returns:
            TransferStatus: The current status.
        """
        return TransferStatus(self._inner.status())

    def error(self) -> TransferError | None:
        """Return why the transfer failed.

        Returns:
            TransferError | None: The failure, or ``None`` while the transfer
                has not failed.
        """
        error = self._inner.error()
        if error is None:
            return None
        kind, message = error
        return TransferError(message, kind)

    def progress(self) -> TransferProgress:
        """Return how far the transfer has come.

        Returns:
            TransferProgress: Bytes and files done and in total.
        """
        return TransferProgress(*self._inner.progress())

    def is_finished(self) -> bool:
        """Return whether the transfer is done or has failed.

        Returns:
            bool: Whether the transfer has finished.
        """
        return self._inner.is_finished()

    def wait(self, timeout: float | None = None) -> None:
        """Block until the transfer has finished; the GIL is released.

        Do not wait inside a callback: it holds a callback worker for as long
        as it waits.

        Args:
            timeout (float | None, optional): Seconds to wait at most; ``None``
                waits without limit. Defaults to None.

        Raises:
            TimeoutError: If the time ran out; the transfer keeps running.
            TransferError: If the transfer failed.
        """
        if not self._inner.wait(timeout):
            raise TimeoutError("the transfer did not finish in time")
        error = self.error()
        if error is not None:
            raise error

    def cancel(self, reason: str | None = None) -> None:
        """Cancel the transfer, or withdraw an offer that is not answered yet.

        Calling it again, or after the transfer has finished, has no effect.

        Args:
            reason (str | None, optional): Told to the other side. Defaults to
                None.
        """
        self._inner.cancel(reason)

    def path(self) -> Path | None:
        """Return where a received transfer is saved.

        Returns:
            Path | None: The file, or the folder of several files; ``None`` for
                data received into memory and for sent transfers.
        """
        path = self._inner.path()
        return None if path is None else Path(path)

    def take_data(self) -> bytes | None:
        """Take the data of a transfer received into memory, once.

        Returns:
            bytes | None: The data once the transfer is done, otherwise
                ``None``.
        """
        return self._inner.take_data()

    def meta(self) -> Any:
        """Return the metadata sent with the transfer.

        Returns:
            Any: The metadata, or ``None`` for an endpoint without metadata.
        """
        return self._inner.meta()

    def __repr__(self) -> str:
        return f"TransferHandle(name={self.name()!r}, status={self.status().name})"


class OfferedFile(NamedTuple):
    """One file of an offer."""

    #: The name the client gave the file: a relative path with ``/`` between
    #: folders. The library has checked that it stays inside a folder it is
    #: saved to.
    name: str
    #: The size in bytes; the client cannot send more.
    size: int
    #: The time the file was last modified on the client, in seconds since
    #: the epoch, if known.
    modified: float | None


class TransferOffer:
    """A transfer the client offers, not answered yet.

    Answer it inside the receiver function by accepting or rejecting it; an
    offer left unanswered when the function returns is rejected.
    """

    __slots__ = ("_inner",)

    def __init__(self, inner: _TransferOffer) -> None:
        self._inner = inner

    def name(self) -> str:
        """Return the file name, or the name of the set for several files.

        Returns:
            str: The name the client gave the transfer.
        """
        return self._inner.name()

    def files(self) -> list[OfferedFile]:
        """Return the files of the offer; there is at least one.

        Returns:
            list[OfferedFile]: Name, size and modification time of each file.
        """
        return [OfferedFile(*file) for file in self._inner.files()]

    def total_size(self) -> int | None:
        """Return the size of all files together; the client cannot send more.

        Returns:
            int | None: The size in bytes.
        """
        return self._inner.total_size()

    def meta(self) -> Any:
        """Return the metadata the client sent with the offer.

        Returns:
            Any: The metadata, or ``None`` for an endpoint without metadata.
        """
        return self._inner.meta()

    def accept_file(self, path: str | PathLike[str]) -> TransferHandle:
        """Accept a single-file offer into the file at ``path``.

        The data goes to a temporary file next to ``path``, which replaces any
        existing file only once the transfer is complete. Missing folders are
        created and the client's modification time is kept.

        Args:
            path (str | PathLike[str]): Where to save the file.

        Returns:
            TransferHandle: The transfer.

        Raises:
            ValueError: If the offer has several files; it stays open.
            RuntimeError: If the offer was already answered.
        """
        return TransferHandle(self._inner.accept_file(fspath(path)))

    def accept_dir(self, dir: str | PathLike[str]) -> TransferHandle:  # noqa: A002
        """Accept the offer into the folder ``dir``, each file under its own name.

        Names like ``run42/0001.tiff`` create subfolders. If the transfer
        fails, files already complete stay.

        Args:
            dir (str | PathLike[str]): The folder to save the files in.

        Returns:
            TransferHandle: The transfer.

        Raises:
            RuntimeError: If the offer was already answered.
        """
        return TransferHandle(self._inner.accept_dir(fspath(dir)))

    def accept_memory(self, limit: int) -> TransferHandle:
        """Accept a single-file offer into memory, up to ``limit`` bytes.

        Take the data with :meth:`TransferHandle.take_data` once it is done.
        An offer larger than ``limit`` is rejected and the handle fails.

        Args:
            limit (int): The largest size accepted.

        Returns:
            TransferHandle: The transfer.

        Raises:
            ValueError: If the offer has several files; it stays open.
            RuntimeError: If the offer was already answered.
        """
        return TransferHandle(self._inner.accept_memory(limit))

    def reject(self, reason: str = "") -> None:
        """Reject the offer; the client's transfer fails with ``reason``.

        Args:
            reason (str, optional): Told to the client. Defaults to "".

        Raises:
            RuntimeError: If the offer was already answered.
        """
        self._inner.reject(reason)


__all__ = [
    "OfferedFile",
    "TransferOffer",
    "TransferError",
    "TransferErrorKind",
    "TransferHandle",
    "TransferProgress",
    "TransferStatus",
]
