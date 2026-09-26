"""Preview or remove staged test packages after all test processes have stopped."""

import argparse
from pathlib import Path
import shutil
import sys

from tests.preparation import ARTIFACTS


def clean_runtime_packages(directory: Path, *, delete: bool = False) -> int:
    """Clean direct runtime-* directories, retaining other artifacts and symlinks."""
    if directory.is_symlink():
        print(f"Refusing symlink artifact directory: {directory}", file=sys.stderr)
        return 1
    failures = 0
    count = 0
    for path in sorted(directory.glob("runtime-*")):
        if path.is_symlink():
            print(f"Skipping symlink: {path}", file=sys.stderr)
            failures += 1
            continue
        if not path.is_dir():
            continue
        if delete:
            try:
                shutil.rmtree(path)
            except OSError as error:
                print(f"Could not remove {path}: {error}", file=sys.stderr)
                failures += 1
                continue
            print(f"Removed {path}")
        else:
            print(f"Would remove {path}")
        count += 1
    print(f"{'Removed' if delete else 'Found'} {count} staged package directories.")
    return int(failures != 0)


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__,
        epilog="This command does not detect active sessions. Stop pytest and its subprocesses before --delete.",
    )
    parser.add_argument("--delete", action="store_true", help="remove the directories (default: preview only)")
    args = parser.parse_args()
    try:
        return clean_runtime_packages(ARTIFACTS, delete=args.delete)
    except OSError as error:
        print(f"Could not inspect {ARTIFACTS}: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
