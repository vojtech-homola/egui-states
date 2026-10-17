"""Import and exercise Python bindings emitted by the Rust generator."""

import dataclasses
import importlib
import importlib.util
import json
import math
from pathlib import Path
import sys

import pytest
from tests.preparation import ROOT, run

MANIFEST = ROOT / "tests/generated-code-consumer/Cargo.toml"


@pytest.fixture(scope="module")
def codegen_probe():
    metadata = json.loads(run(["cargo", "metadata", "--no-deps", "--format-version=1", "--locked"]))
    # Share the user's configured target directory; this Cargo invocation runs
    # from pytest, after preparation has released Cargo's build lock.
    command = [
        "cargo",
        "build",
        "--manifest-path",
        MANIFEST,
        "--target-dir",
        metadata["target_directory"],
        "--bin",
        "codegen-probe",
        "--message-format=json-render-diagnostics",
    ]
    output = run(command)
    artifacts = []
    for line in output.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            print(line)
            continue
        if message.get("reason") == "compiler-artifact" and message["target"]["name"] == "codegen-probe":
            artifacts.append(message["executable"])
        elif message.get("reason") == "compiler-message":
            print(message["message"].get("rendered", ""))
    assert len(artifacts) == 1, output
    return Path(artifacts[0]), metadata["target_directory"]


def import_bindings(name, package):
    """Import the generated Python package in `package` under a unique module name."""
    spec = importlib.util.spec_from_file_location(
        name, package / "__init__.py", submodule_search_locations=[str(package)]
    )
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def test_generated_python_is_usable(codegen_probe, tmp_path):
    probe, _ = codegen_probe
    run([probe, tmp_path], timeout=15)
    module = import_bindings("compiled_test_bindings", tmp_path / "python")
    server = module.StatesServer(signals_workers=1)
    payload = server.states.payload.get()
    assert payload.title == "hello 🦀"
    assert payload.fixed == [0, 17, 65535]
    assert payload.nested.enabled is True
    assert payload.choice.value == 71
    assert server.states.mapping.get() == {1: 10, 3: 30, 20: 200}
    assert not server.is_running()
    server.stop()


ESCAPED = 'quote " backslash \\ tab \t newline \n control \x01 delete \x7f'


@pytest.fixture(scope="module")
def awkward_bindings(codegen_probe, tmp_path_factory):
    """Generate the awkward schemas from tests/generated-code-consumer/awkward.rs."""
    probe, _ = codegen_probe
    directory = tmp_path_factory.mktemp("awkward")
    run([probe, directory, "awkward"], timeout=15)
    return directory


def awkward_server(directory, kind):
    module = import_bindings(f"awkward_{kind}_bindings", directory / kind)
    server = module.StatesServer(signals_workers=1)
    states = vars(server.states)

    def state(prefix):
        # Escaping may rename a keyword (for example to `global_`); accept any such spelling.
        matches = [value for name, value in states.items() if name.rstrip("_") == prefix]
        assert len(matches) == 1, f"no single state for {prefix!r}: {sorted(states)}"
        return matches[0]

    return module, server, state


def test_python_keywords_as_generated_names(awkward_bindings):
    module, server, state = awkward_server(awkward_bindings, "keywords")
    keyword = importlib.import_module(f"{module.__name__}.enums").Keyword
    assert [member.value for member in keyword] == [0, 1, 2]
    assert state("keyword").get() == keyword(1)
    reserved = state("reserved").get()
    assert dataclasses.astuple(reserved) == (3, True)
    assert state("global").get() == 1
    state("keyword").set(keyword(0))
    state("reserved").set(type(reserved)(7, False))
    assert state("keyword").get() == keyword(0)
    assert dataclasses.astuple(state("reserved").get()) == (7, False)
    server.stop()


def test_special_literals_as_generated_initial_values(awkward_bindings):
    _, server, state = awkward_server(awkward_bindings, "literals")
    assert state("escaped").get() == ESCAPED
    assert math.isnan(state("not_a_number").get())
    assert state("positive_infinity").get() == math.inf
    assert state("negative_infinity").get() == -math.inf
    assert math.isnan(state("single_not_a_number").get())
    state("escaped").set(ESCAPED[::-1])
    assert state("escaped").get() == ESCAPED[::-1]
    server.stop()


def test_unit_struct_values(awkward_bindings):
    _, server, state = awkward_server(awkward_bindings, "unit")
    marker = state("marker").get()
    assert dataclasses.astuple(marker) == ()
    state("marker").set(marker)
    assert state("marker").get() == marker
    server.stop()
