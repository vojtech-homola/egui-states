import threading
import traceback
from collections.abc import Callable
from typing import Any

from egui_pysync.typing import SteteServerCoreBase

type ArgParser = Callable[[Any], Any] | bool


class _SignalsManager:
    def __init__(
        self,
        server: SteteServerCoreBase,
        workers: int,
        error_handler: Callable[[Exception], None] | None,
    ):
        self._callbacks: dict[int, tuple[list[Callable], ArgParser]] = {}
        self._server = server

        self._workers_count = workers
        self._workers: list[threading.Thread] = []
        self._error_handler = error_handler or self._default_error_handler

    def register_value(self, value_id: int, arg_parser: ArgParser = False) -> None:
        self._callbacks[value_id] = ([], arg_parser)

    def close_registration(self) -> None:
        for i in range(self._workers_count):
            worker = threading.Thread(target=self._run, args=(i,), daemon=True, name=f"signals_worker_{i}")
            self._workers.append(worker)
            worker.start()

    def check_workers(self) -> None:
        for i, worker in enumerate(self._workers):
            if not worker.is_alive():
                self._workers[i] = threading.Thread(
                    target=self._run, args=(i,), daemon=True, name=f"signals_worker_{i}"
                )
                self._workers[i].start()

    def _run(self, thread_id) -> None:
        while True:
            ind, arg = self._server.value_get_signal(thread_id)
            callbacks, arg_parser = self._callbacks.get(ind, (None, False))
            if callbacks:
                if arg_parser is False:
                    for callback in callbacks:
                        try:
                            callback(arg)
                        except Exception as e:
                            self._error_handler(e)
                elif arg_parser is True:
                    for callback in callbacks:
                        try:
                            callback()
                        except Exception as e:
                            self._error_handler(e)
                else:
                    for callback in callbacks:
                        try:
                            callback(arg_parser(arg))
                        except Exception as e:
                            self._error_handler(e)

            else:
                error = IndexError(f"Signal with index {ind} not found.")
                self._error_handler(error)

    @staticmethod
    def _default_error_handler(_e: Exception) -> None:
        traceback.print_exc()

    def set_error_handler(self, error_handler: Callable[[Exception], None] | None) -> None:
        self._error_handler = error_handler or self._default_error_handler

    def add_callback(self, value_id: int, callback: Callable) -> None:
        if value_id in self._callbacks:
            self._callbacks[value_id][0].append(callback)
            self._server.value_set_register(value_id, True)
        else:
            raise RuntimeError(f"Signal with index {value_id} not found.")

    def remove_callback(self, value_id: int, callback: Callable) -> None:
        if value_id in self._callbacks:
            if callback in self._callbacks[value_id]:
                self._callbacks[value_id][0].remove(callback)
                if not self._callbacks[value_id]:
                    self._server.value_set_register(value_id, False)
        else:
            raise RuntimeError(f"Signal with index {value_id} not found.")

    def clear_callbacks(self, value_id: int) -> None:
        if value_id in self._callbacks:
            self._callbacks[value_id][0].clear()
            self._server.value_set_register(value_id, False)
