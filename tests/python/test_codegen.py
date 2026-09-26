"""Import and exercise Python bindings emitted by the Rust generator."""

import importlib.util
import json
import os
from pathlib import Path
import sys

import pytest
from tests.preparation import ROOT, run

MANIFEST = ROOT / "tests/generated-code-consumer/Cargo.toml"
FILES = [
    "python/__init__.py",
    "python/structs.py",
    "python/enums.py",
]


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


def snapshot(directory):
    return {file: (directory / file).read_bytes() for file in FILES}


def test_generated_python_is_usable_and_equivalent_inputs_are_deterministic(codegen_probe, tmp_path):
    probe, _ = codegen_probe
    first, second = tmp_path / "first", tmp_path / "second"
    run([probe, first], timeout=15)
    run([probe, second, "reverse"], timeout=15)
    assert snapshot(first) == snapshot(second)
    spec = importlib.util.spec_from_file_location(
        "compiled_test_bindings", first / "python/__init__.py", submodule_search_locations=[str(first / "python")]
    )
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    server = module.StatesServer(signals_workers=1)
    payload = server.states.payload.get()
    assert payload.title == "hello 🦀"
    assert payload.fixed == [0, 17, 65535]
    assert payload.nested.enabled is True
    assert payload.choice.value == 71
    assert server.states.mapping.get() == {1: 10, 3: 30, 20: 200}
    assert not server.is_running()
    server.stop()


def test_regeneration_does_not_rewrite_any_unchanged_file(codegen_probe, tmp_path):
    probe, _ = codegen_probe
    run([probe, tmp_path], timeout=15)
    before = snapshot(tmp_path)
    for file in FILES:
        os.utime(tmp_path / file, (946684800, 946684800))
    timestamps = {file: (tmp_path / file).stat().st_mtime_ns for file in FILES}
    run([probe, tmp_path, "reverse"], timeout=15)
    assert snapshot(tmp_path) == before
    assert {file: (tmp_path / file).stat().st_mtime_ns for file in FILES} == timestamps
