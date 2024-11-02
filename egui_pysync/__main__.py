"""Generate the python files for generated rust binding."""

import io
import os
import sys


def _write_enums(input_path: str, output_path: str) -> bool:
    input_file = os.path.join(input_path, "enums.rs")
    if not os.path.exists(input_file):
        raise FileNotFoundError(f"File not found: {input_file}")

    enums_list = []
    with open(os.path.join(output_path, "enums.py"), "w", encoding="utf-8") as file:  # noqa: PLR1702
        file.write("# ruff: noqa: D101\n")
        file.write("from enum import Enum\n")

        with open(input_file, encoding="utf-8") as enums_file:
            lines = enums_file.readlines()

        while len(lines) > 0:
            line = lines.pop(0)

            if "pub enum" in line or "pub(crate) enum" in line:
                file.write("\n\n")
                enum_name = line.split()[2]
                enums_list.append(enum_name)
                file.write(f"class {enum_name}(Enum):\n")
                counter = 0
                while True:
                    line = lines.pop(0)
                    if "#" in line:
                        continue
                    elif "}" in line:
                        break
                    else:
                        line = line.replace(",", "").strip()
                        if "=" in line:
                            file.write(f"    {line}\n")
                        else:
                            file.write(f"    {line} = {counter}\n")
                            counter += 1

    return True


def _write_types(input_path: str, output_path: str) -> bool:
    input_file = os.path.join(input_path, "custom.rs")
    if not os.path.exists(input_file):
        raise FileNotFoundError(f"File not found: {input_file}")

    with open(input_file, encoding="utf-8") as enums_file:
        lines = enums_file.readlines()

    to_write = []

    def parse_types(lines: list[str]):
        line = lines[0]
        if "//" in line:
            if "class" in line:
                text = line.replace("//", "").strip()
                to_write.append(f"\n{text}\n")
                i = 1
                while True:
                    if "//" in lines[i]:
                        text = lines[i].replace("//", "").strip()
                        to_write.append(f"    {text}\n")
                        i += 1
                    else:
                        break

            else:
                text = line.replace("//", "").strip()
                to_write.append(f"\n{text}\n")

        else:
            name = line.split("struct ")[1].split("(")[0].split("{")[0].strip()
            to_write.append(f"\ntype {name} = Any\n")

    for i, line in enumerate(lines):
        if "#[derive" in line and "//" not in line:
            parse_types(lines[i + 1 :])

    with open(os.path.join(output_path, "types.py"), "w", encoding="utf-8") as file:
        file.write("# ruff: noqa: UP013 F403 F405 D101 E302 E305\n")
        file.write("from typing import *  # type: ignore\n")
        file.write("from collections.abc import *  # type: ignore\n\n")

        for line in to_write:
            file.write(line)

    return True


type_map = {
    "u8": "int",
    "u16": "int",
    "u32": "int",
    "u64": "int",
    "i8": "int",
    "i16": "int",
    "i32": "int",
    "i64": "int",
    "f32": "float",
    "f64": "float",
    "bool": "bool",
    "String": "str",
}


def _parse_value(value: str) -> str:
    if value in type_map:
        return type_map[value]

    if value == "()":
        return ""

    if "custom::" in value:
        return value.replace("custom::", "types.")

    if value[0] == "[" and value[-1] == "]":
        val = value[1:-1]
        if ";" in val:
            typ_val = val.split(";")[0].strip()
            nums = val.split(";")[1].strip()
            typ_val = _parse_value(typ_val)
            text = ", ".join([typ_val] * int(nums))
            return f"tuple[{text}]"
        else:
            vals = val.split(",")
            text = ", ".join([_parse_value(v.strip()) for v in vals])
            return f"tuple[{text}]"

    if value[0] == "(" and value[-1] == ")":
        vals = value[1:-1].split(",")
        text = ", ".join([_parse_value(v.strip()) for v in vals])
        return f"tuple[{text}]"

    raise ValueError(f"Unknown value: {value}")


class _Struct:
    def __init__(self, lines: list[str]) -> None:
        self._name = lines[0].split("struct ")[1].split(" ")[0]

        self._structs_line = {}
        self._items_line = {}

        self._structs_names = {}

        self._order = []
        self._original_items = {}

        original_types = {}
        for line in lines:
            if "Arc" in line:
                name, text = self._parse_item(line)
                if name:
                    self._items_line[name] = text
                    type_string = line.split("Arc<")[1].split(">,")[0]
                    if "<" in type_string:
                        type_string = type_string.split("<")[1].split(">")[0]
                        original_types[name] = type_string

            elif not line.strip():
                continue
            elif "}" in line:
                break
            elif "{" in line:
                continue
            elif "Arc" not in line and "pub" in line:
                name = line.strip().split(":")[0].split(" ")[-1].strip()
                struct_name = line.split(":")[1].split(",")[0].strip()
                self._structs_names[name] = struct_name
                self._structs_line[name] = f"        self.{name} = {struct_name}(c)\n"

        started = False
        for line in lines:
            if not started:
                if "Self" in line and "fn new" not in line:
                    started = True
                continue
            if "//" in line or not line.strip():
                continue
            if "}" in line:
                break

            name = line.split(":")[0].strip()
            self._order.append(name)
            if "::new(c)," not in line:
                original = line.split(": ")[1].strip()
                original = original.replace("),", ");")
                if name in original_types and "<" not in original:
                    idx = original.find("(")
                    original = original[:idx] + f"::<{original_types[name]}>" + original[idx:]

                self._original_items[name] = original

    def write(self, file: io.TextIOWrapper, structs: dict, is_root: bool = False) -> None:
        for struct in self._structs_line:
            name = self._structs_names[struct]
            if name in structs:
                struct_to_process = structs.pop(name)
                struct_to_process.write(file, structs)

        if is_root:
            file.write(f"\nclass {self._name}(structures._MainStatesBase):\n")
            file.write("    def __init__(self, update: Callable[[float | None], None]):\n")
            file.write("        self._update = update\n")
            file.write("        c = structures._Counter()\n\n")
        else:
            file.write(f"\nclass {self._name}(structures._StatesBase):\n")
            file.write("    def __init__(self, c: structures._Counter):\n")

        for name in self._order:
            if name in self._items_line:
                file.write(self._items_line[name])
            else:
                file.write(self._structs_line[name])

        file.write("\n")

    def write_server_state(self, file: io.TextIOWrapper, structs: dict[str, "_Struct"]) -> None:
        for item in self._order:
            if item in self._original_items:
                file.write(f"    {self._original_items[item]}\n")
            else:
                file.write(f"    //{self._structs_names[item]}\n")
                structs[self._structs_names[item]].write_server_state(file, structs)

    @staticmethod
    def _parse_item(line: str) -> tuple[str, str]:
        if "//" in line:
            return "", ""

        name = line.strip().split(":")[0].split(" ")[-1]
        to_write = ""

        try:
            type_string = line.split("<")[2].split(">")[0]
        except IndexError:
            type_string = ""

        if "Value<" in line:
            val_type = _parse_value(type_string)
            to_write = f"        self.{name} = structures.Value[{val_type}](c)\n"

        elif "ValueEnum" in line:
            enum_str = type_string.replace("::", ".")
            to_write = f"        self.{name} = structures.ValueEnum(c, {enum_str})\n"

        elif "ValueStatic" in line:
            val_type = _parse_value(type_string)
            to_write = f"        self.{name} = structures.ValueStatic[{val_type}](c)\n"

        elif "Signal" in line:
            val_type = _parse_value(type_string)
            if val_type:
                to_write = f"        self.{name} = structures.Signal[{val_type}](c)\n"
            else:
                to_write = f"        self.{name} = structures.SignalEmpty(c)\n"

        elif "ValueImage" in line:
            to_write = f"        self.{name} = structures.ValueImage(c)\n"

        elif "ValueDict" in line:
            key_type = _parse_value(type_string.split(",")[0].strip())
            val_type = _parse_value(type_string.split(",")[1].strip())
            to_write = f"        self.{name} = structures.ValueDict[{key_type}, {val_type}](c)\n"

        elif "ValueList" in line:
            val_type = _parse_value(type_string)
            to_write = f"        self.{name} = structures.ValueList[{val_type}](c)\n"

        elif "ValueGraphs" in line:
            to_write = f"        self.{name} = structures.ValueGraphs(c)\n"

        else:
            raise ValueError(f"Unknown type: {line}")

        return name, to_write


def _write_states(input_path: str, output_path: str, server_path: str) -> None:  # noqa: PLR0912, PLR0915
    with open(os.path.join(input_path, "states.rs"), encoding="utf-8") as state_file:
        lines = state_file.readlines()

    structs: dict[str, _Struct] = {}
    for i, line in enumerate(lines):
        if "struct" in line:
            struct = _Struct(lines[i:])
            structs[struct._name] = struct

    with open(os.path.join(output_path, "states.py"), "w", encoding="utf-8") as file:
        file.write("# ruff: noqa: D107 D101\n")
        file.write("from collections.abc import Callable\n\n")
        file.write("from egui_pysync import structures\n\n")
        if os.path.exists(os.path.join(output_path, "enums.py")):
            file.write("from expert_ui import enums\n")
        if os.path.exists(os.path.join(output_path, "types.py")):
            file.write("from expert_ui import types\n")
        file.write("\n")

        structs_copy = structs.copy()
        structs["States"].write(file, structs, is_root=True)

        file.write("    def update(self, duration: float | None = None) -> None:\n")
        file.write('        """Update the UI.\n\n')
        file.write("        Args:\n")
        file.write("            duration: The duration of the update.\n")
        file.write('        """\n')
        file.write("        self._update(duration)\n")

    with open(os.path.join(server_path, "states.rs"), "w", encoding="utf-8") as file:
        head = [
            "#![allow(unused_imports)]\n",
            "use types::{custom, enums};\n\n",
            "use egui_pyserver::{Signal, Value, ValueDict, ValueEnum, ValueImage, ValueStatic, ValuesCreator};\n\n"
        ]
        for line in head:
            file.write(line)

        file.write("pub(crate) fn create_states(c: &mut ValuesCreator) {\n")
        structs_copy["States"].write_server_state(file, structs_copy)
        file.write("}\n")


if __name__ == "__main__":
    command_type = sys.argv[1]
    input_path = os.path.join(os.getcwd(), sys.argv[2])
    output_path = os.path.join(os.getcwd(), sys.argv[3])

    if command_type == "enums":
        _write_enums(input_path, output_path)
    elif command_type == "types":
        _write_types(input_path, output_path)
    elif command_type == "states":
        server_path = os.path.join(os.getcwd(), sys.argv[4])
        _write_states(input_path, output_path, server_path)
    else:
        raise ValueError(f"Unknown command: {command_type}")
