# ruff: noqa: PLC2801
from enum import Enum
from typing import Any, Self


class _FastEnum(Enum):
    def __init_subclass__(cls):
        super().__init_subclass__()
        cls._member_list = tuple(cls)

    @classmethod
    def from_index(cls, index) -> Self:
        return cls._member_list[index]

    def index(self) -> int:
        return self._member_list.index(self)


class _CustomStruct:
    __getitem__ = object.__getattribute__

    def _get_values(self) -> list[Any]:
        return [self.__getattribute__(name) for name in self.__annotations__.keys()]
