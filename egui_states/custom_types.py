# ruff: noqa: PLC2801 D101 D102 D105 D107
from enum import Enum
from typing import Any, Self

from egui_states._core import PyObjectType


class FastEnum(Enum):
    def __init_subclass__(cls):
        super().__init_subclass__()
        cls._member_list = tuple(cls)

    @classmethod
    def from_index(cls, index) -> Self:
        return cls._member_list[index]

    def index(self) -> int:
        return self._member_list.index(self)


# from dataclasses import dataclass


class CustomStruct:
    __getitem__ = object.__getattribute__

    def _get_values(self) -> list[Any]:
        return [self.__getattribute__(name) for name in self.__annotations__.keys()]

    # @classmethod
    # def _get_type(cls) -> PyObjectType:
    #     field_types = [
    #         cls.__annotations__[name].get_type()
    #         for name in cls.__annotations__.keys()
    #     ]
    #     return PyObjectType.struct(field_types)


# @dataclass
# class TestStruct(CustomStruct):
#     a: int
#     b: float
#     c: str


class U8:
    @staticmethod
    def get_type() -> PyObjectType:
        return PyObjectType.u8()


class U16:
    @staticmethod
    def get_type() -> PyObjectType:
        return PyObjectType.u16()


class U32:
    @staticmethod
    def get_type() -> PyObjectType:
        return PyObjectType.u32()


class U64:
    @staticmethod
    def get_type() -> PyObjectType:
        return PyObjectType.u64()


class I8:
    @staticmethod
    def get_type() -> PyObjectType:
        return PyObjectType.i8()


class I16:
    @staticmethod
    def get_type() -> PyObjectType:
        return PyObjectType.i16()


class I32:
    @staticmethod
    def get_type() -> PyObjectType:
        return PyObjectType.i32()


class I64:
    @staticmethod
    def get_type() -> PyObjectType:
        return PyObjectType.i64()


class F32:
    @staticmethod
    def get_type() -> PyObjectType:
        return PyObjectType.f32()


class F64:
    @staticmethod
    def get_type() -> PyObjectType:
        return PyObjectType.f64()


class Bool:
    @staticmethod
    def get_type() -> PyObjectType:
        return PyObjectType.boolean()


class String:
    @staticmethod
    def get_type() -> PyObjectType:
        return PyObjectType.string()


class Empty:
    @staticmethod
    def get_type() -> PyObjectType:
        return PyObjectType.empty()


class En:
    def __init__(self, enum_: type[FastEnum]):
        self.enum = enum_

    def get_type(self) -> PyObjectType:
        return PyObjectType.enum_(self.enum)


class Tuple:
    def __init__(self, elements: list[PyObjectType]):
        self.elements = elements

    def get_type(self) -> PyObjectType:
        return PyObjectType.tuple_(self.elements)


class List:
    def __init__(self, element_type: PyObjectType, size: int):
        self.element_type = element_type
        self.size = size

    def get_type(self) -> PyObjectType:
        return PyObjectType.list_(self.element_type, self.size)


class Vec:
    def __init__(self, element_type: PyObjectType):
        self.element_type = element_type

    def get_type(self) -> PyObjectType:
        return PyObjectType.vec(self.element_type)


class Map:
    def __init__(self, key_type: PyObjectType, value_type: PyObjectType):
        self.key_type = key_type
        self.value_type = value_type

    def get_type(self) -> PyObjectType:
        return PyObjectType.map(self.key_type, self.value_type)


class Clas:
    def __init__(self, class_type: type[CustomStruct], elements: list[PyObjectType]):
        self.class_type = class_type
        self.elements = elements

    def get_type(self) -> PyObjectType:
        return PyObjectType.class_(self.elements, self.class_type)
