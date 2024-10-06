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
    for i, line in enumerate(lines):
        if "#[derive" not in line:
            continue

        inner_lines = lines[i + 1 :]
        firs_line = inner_lines[0]
        if "struct" in firs_line and "{" in firs_line:
            # TODO: Implement structs
            # name = firs_line.replace("pub(crate)", "").replace("pub", "").replace("struct", "").replace("{", "").strip()
            raise NotImplementedError("Normal structs are not supported yet")

        else:
            name = firs_line.split("(")[0].replace("pub(crate)", "").replace("pub", "").replace("struct", "").strip()
            types_str = firs_line.split("(")[1].split(")")[0].split(", ")
            types = [_parse_value(t.replace("pub", "").strip()) for t in types_str]
            text = f"type {name} = tuple[{', '.join(types)}]"
            to_write.append(text)

    with open(os.path.join(output_path, "types.py"), "w", encoding="utf-8") as file:
        # file.write("# ruff: noqa: D101\n\n")
        for line in to_write:
            file.write(f"{line}\n")

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

        self._structs = {}
        self._structs_names = {}
        self._items = {}
        self._order = []
        self._original_items = {}
        self._original_types = {}

        for line in lines:
            if "Arc" in line:
                name, text = self._parse_item(line)
                if name:
                    self._items[name] = text
                    type_string = line.split("Arc<")[1].split(">,")[0]
                    if "<" in type_string:
                        type_string = type_string.split("<")[1].split(">")[0]
                        self._original_types[name] = type_string

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
                self._structs[name] = f"        self.{name} = {struct_name}()\n"

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
                if name in self._original_types and "<" not in original:
                    idx = original.find("(")
                    original = original[:idx] + f"::<{self._original_types[name]}>" + original[idx:]

                self._original_items[name] = original

    def set_id(self, id_counter: int, structs: dict) -> int:
        for item in self._order:
            if item in self._structs:
                id_counter = structs[self._structs_names[item]].set_id(id_counter, structs)
            else:
                self._items[item] = self._items[item].replace("*", str(id_counter))
                id_counter += 1

        return id_counter

    def write(self, file: io.TextIOWrapper, structs: dict, head: list[str] | None = None):
        for struct in self._structs:
            name = self._structs_names[struct]
            structs[name].write(file, structs)

        if head:
            for line in head:
                file.write(line)
        else:
            file.write(f"\nclass {self._name}(structures._StatesBase):\n")
            file.write("    def __init__(self):")

        for to_write in self._structs.values():
            file.write(to_write)

        file.write("\n")

        for item in self._items.values():
            file.write(item)

        file.write("\n")

    def write_server_state(self, file: io.TextIOWrapper, structs: dict) -> None:
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
            to_write = f"        self.{name} = structures.Value[{val_type}](*)\n"

        elif "ValueEnum" in line:
            enum_str = type_string.replace("::", ".")
            to_write = f"        self.{name} = structures.ValueEnum(*, {enum_str})\n"

        elif "ValueStatic" in line:
            val_type = _parse_value(type_string)
            to_write = f"        self.{name} = structures.ValueStatic[{val_type}](*)\n"

        elif "Signal" in line:
            val_type = _parse_value(type_string)
            if val_type:
                to_write = f"        self.{name} = structures.Signal[{val_type}](*)\n"
            else:
                to_write = f"        self.{name} = structures.SignalEmpty(*)\n"

        elif "ImageValue" in line:
            to_write = f"        self.{name} = structures.ValueImage(*)\n"

        elif "ValueDict" in line:
            key_type = _parse_value(type_string.split(",")[0].strip())
            val_type = _parse_value(type_string.split(",")[1].strip())
            to_write = f"        self.{name} = structures.ValueDict[{key_type}, {val_type}](*)\n"

        elif "ValueList" in line:
            val_type = _parse_value(type_string)
            to_write = f"        self.{name} = structures.ValueList[{val_type}](*)\n"

        elif "ValueGraph" in line:
            to_write = f"        self.{name} = structures.ValueGraph(*)\n"

        else:
            raise ValueError(f"Unknown type: {line}")

        return name, to_write


def _write_states(input_path: str, output_path: str, server_path: str) -> None:  # noqa: PLR0912, PLR0915
    id_counter = 10  # first 10 ids are reserved for the special values
    with open(os.path.join(input_path, "states.rs"), encoding="utf-8") as state_file:
        lines = state_file.readlines()

    structs: dict[str, _Struct] = {}
    for i, line in enumerate(lines):
        if "struct" in line:
            struct = _Struct(lines[i:])
            structs[struct._name] = struct

    structs["States"].set_id(id_counter, structs)

    with open(os.path.join(output_path, "states.py"), "w", encoding="utf-8") as file:
        file.write("# ruff: noqa: D107 D101\n")
        file.write("from egui_pysync import structures\n\n")
        if os.path.exists(os.path.join(output_path, "enums.py")):
            file.write("from expert_ui import enums\n")
        if os.path.exists(os.path.join(output_path, "types.py")):
            file.write("from expert_ui import types\n")
        file.write("\n")

        head = [
            "\nclass States(structures._StatesBase):\n",
            "    def __init__(self):\n",
        ]

        structs["States"].write(file, structs, head)

        file.write("        self._updater = structures._Updater()\n")

        file.write("\n    def update(self, duration: float | None = None) -> None:\n")
        file.write('        """Update the UI.\n\n')
        file.write("        Args:\n")
        file.write("            duration: The duration of the update.\n")
        file.write('        """\n')
        file.write("        self._updater.update(duration)\n")

    with open(os.path.join(server_path, "states.rs"), "w", encoding="utf-8") as file:
        head = [
            "#![allow(unused_imports)]\n",
            "use types::{custom, enums};\n\n",
            "use egui_pysync_server::{\n",
            "    ImageValue, Signal, Value, ValueDict, ValueEnum, ValueStatic, ValuesCreator,\n",
            "};\n\n",
        ]
        for line in head:
            file.write(line)

        file.write("pub(crate) fn create_states(c: &mut ValuesCreator) {\n")
        structs["States"].write_server_state(file, structs)
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
