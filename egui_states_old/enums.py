from enum import Enum
from typing import Self


class _FastEnum(Enum):
    def __init_subclass__(cls):
        super().__init_subclass__()
        cls._member_list = tuple(cls)

    @classmethod
    def by_index(cls, index) -> Self:
        return cls._member_list[index]

    def index(self) -> int:
        return self._member_list.index(self)
