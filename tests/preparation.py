"""Build and locate test artifacts without changing the installed package."""

import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import sysconfig
import tempfile

ROOT = Path(__file__).resolve().parents[1]
ARTIFACTS = ROOT / "tests" / "build-artifacts"


def run(command, *, timeout=600, cwd=ROOT, env=None):
    """Run a bounded command, retaining diagnostics and terminating its children."""
    process = subprocess.Popen(
        [str(arg) for arg in command],
        cwd=cwd,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        start_new_session=os.name != "nt",
    )
    try:
        output, _ = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        if os.name == "nt":
            subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"], capture_output=True)
        else:
            os.killpg(process.pid, signal.SIGKILL)
        output, _ = process.communicate()
        raise RuntimeError(f"Timed out after {timeout}s: {command!r}\n{output}") from None
    if process.returncode:
        raise RuntimeError(f"Exit {process.returncode}: {command!r}\n{output}")
    return output


def build(*packages, bins=False):
    """Return Cargo compiler artifacts, respecting the caller's target/offline settings."""
    command = ["cargo", "build", "--locked", "--message-format=json-render-diagnostics"]
    for package in packages:
        command.extend(["-p", package])
    if bins:
        command.append("--bins")
    output = run(command, env={**os.environ, "PYO3_PYTHON": sys.executable})
    artifacts = {}
    for line in output.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            print(line, file=sys.stderr)
            continue
        if message.get("reason") == "compiler-artifact":
            artifacts[message["target"]["name"]] = message
        elif message.get("reason") == "compiler-message":
            print(message["message"].get("rendered", ""), file=sys.stderr, end="")
    return artifacts


def executable(artifacts, name):
    """Resolve the executable Cargo reported, including target triples and suffixes."""
    path = artifacts[name].get("executable")
    if not path or not Path(path).is_file():
        raise RuntimeError(f"Cargo did not produce executable {name!r}: {artifacts[name]}")
    return Path(path)


def prepare_extension():
    """Stage a fresh extension before any egui_states imports in this process."""
    if "egui_states" in sys.modules or "egui_states._core" in sys.modules:
        raise RuntimeError("egui_states was imported before test preparation; start a fresh pytest process")
    artifacts = build("egui_states_python")
    candidates = [
        Path(path)
        for path in artifacts["egui_states_python"]["filenames"]
        if Path(path).suffix in {".so", ".dylib", ".dll"}
    ]
    if len(candidates) != 1:
        raise RuntimeError(f"Expected one native extension, got {candidates}")
    ARTIFACTS.mkdir(parents=True, exist_ok=True)
    # Keep this ignored artifact until a later cleanup: Windows holds loaded DLLs open.
    stage = Path(tempfile.mkdtemp(prefix="runtime-", dir=ARTIFACTS))
    package = stage / "egui_states"
    shutil.copytree(
        ROOT / "egui_states",
        package,
        ignore=shutil.ignore_patterns("__pycache__", "*.so", "*.pyd", "*.pdb", "*.pyc"),
    )
    extension = package / ("_core" + sysconfig.get_config_var("EXT_SUFFIX"))
    shutil.copy2(candidates[0], extension)
    sys.path.insert(0, str(stage))
    os.environ["PYTHONPATH"] = os.pathsep.join(filter(None, [str(stage), os.environ.get("PYTHONPATH")]))
    import egui_states._core as core

    if Path(core.__file__).resolve() != extension.resolve():
        raise RuntimeError(f"Loaded stale extension: {core.__file__}; expected {extension}")
    return extension
