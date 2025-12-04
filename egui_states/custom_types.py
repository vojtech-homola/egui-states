# ruff: noqa: PLC2801 D101 D102 D105
from enum import Enum
from typing import Any, Self


class FastEnum(Enum):
    def __init_subclass__(cls):
        super().__init_subclass__()
        cls._member_list = tuple(cls)

    @classmethod
    def from_index(cls, index) -> Self:
        return cls._member_list[index]

    @classmethod
    def _get_members(cls) -> list[tuple[str, int]]:
        return [(member.name, member.value) for member in cls._member_list]

    def index(self) -> int:
        return self._member_list.index(self)


class CustomStruct:
    __getitem__ = object.__getattribute__

    def _get_values(self) -> list[Any]:
        return [self.__getattribute__(name) for name in self.__annotations__.keys()]

    @classmethod
    def _field_names(cls) -> list[str]:
        return list(cls.__annotations__.keys())
