# Ganerated by build.rs, do not edit
# ruff: noqa: D107 D101
from collections.abc import Callable

from egui_pysync import structures as sc

from images._core import core


class States(sc._MainStatesBase):
    def __init__(self, update: Callable[[float | None], None]):
        self._update = update
        c = sc._Counter()

        self.image = sc.ValueImage(c)

    def update(self, duration: float | None = None) -> None:
        """Update the UI.

        Args:
            duration (float | None): The duration of the update.
        """
        self._update(duration)
