"""Contain blocking native calls and all callback workers outside pytest."""

from pathlib import Path
import sys

# Append the project for importing test helpers while keeping the staged package
# first on PYTHONPATH. The script directory itself is tests/, not the repo root.
sys.path.append(str(Path(__file__).resolve().parents[1]))
from tests.python.integration_server import run_scenario
import egui_states._core as core

print(f"Integration worker extension: {core.__file__}", flush=True)
assert "runtime-" in str(core.__file__), "integration worker loaded an unstaged extension"
run_scenario(sys.argv[1], sys.argv[2])
