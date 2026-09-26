"""Keep the explicit cleanup command confined to disposable staged packages."""

import pytest

from tests.clean_artifacts import clean_runtime_packages


def test_preview_preserves_packages_and_delete_preserves_other_artifacts(tmp_path, capsys):
    package = tmp_path / "runtime-old" / "egui_states"
    package.mkdir(parents=True)
    extension = package / "_core.so"
    extension.write_bytes(b"staged extension")
    bindings = tmp_path / "egui_states_test_bindings"
    bindings.mkdir()
    source = bindings / "__init__.py"
    source.write_text("# generated bindings\n")
    ordinary_file = tmp_path / "runtime-not-a-directory"
    ordinary_file.write_text("keep\n")

    assert clean_runtime_packages(tmp_path) == 0
    assert "Would remove" in capsys.readouterr().out
    assert extension.read_bytes() == b"staged extension"

    assert clean_runtime_packages(tmp_path, delete=True) == 0
    assert not package.parent.exists()
    assert source.read_text() == "# generated bindings\n"
    assert ordinary_file.read_text() == "keep\n"


def test_cleanup_refuses_symlinks_and_preserves_their_targets(tmp_path, capsys):
    artifacts = tmp_path / "artifacts"
    artifacts.mkdir()
    outside = tmp_path / "outside"
    outside.mkdir()
    sentinel = outside / "important.txt"
    sentinel.write_text("keep\n")
    link = artifacts / "runtime-linked"
    root_link = tmp_path / "linked-artifacts"
    try:
        link.symlink_to(outside, target_is_directory=True)
        root_link.symlink_to(artifacts, target_is_directory=True)
    except OSError as error:
        pytest.skip(f"Creating directory symlinks is unavailable: {error}")

    assert clean_runtime_packages(artifacts, delete=True) == 1
    assert "Skipping symlink" in capsys.readouterr().err
    assert clean_runtime_packages(root_link, delete=True) == 1
    assert "Refusing symlink artifact directory" in capsys.readouterr().err
    assert link.is_symlink()
    assert sentinel.read_text() == "keep\n"


def test_cleanup_reports_locked_packages_and_continues(tmp_path, monkeypatch, capsys):
    from tests import clean_artifacts

    locked = tmp_path / "runtime-a-locked"
    removable = tmp_path / "runtime-b-unused"
    locked.mkdir()
    removable.mkdir()
    original = clean_artifacts.shutil.rmtree

    def remove(path):
        if path == locked:
            raise PermissionError("package is in use")
        original(path)

    monkeypatch.setattr(clean_artifacts.shutil, "rmtree", remove)
    assert clean_runtime_packages(tmp_path, delete=True) == 1
    assert locked.is_dir()
    assert not removable.exists()
    assert "package is in use" in capsys.readouterr().err
