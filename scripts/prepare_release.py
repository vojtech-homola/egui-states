"""Prepare a release: bump the version and regenerate the changelog.

Run from the repository root:

    python scripts/prepare_release.py --bump auto --dry-run   # preview only
    python scripts/prepare_release.py --bump auto             # write the files

The bump is derived from the Conventional Commit titles since the last version
tag: a breaking change (``type!:`` or a ``BREAKING CHANGE:`` footer) bumps the
major version (the minor version before 1.0), ``feat`` bumps the minor version,
and anything else bumps the patch version. A change of ``PROTOCOL_VERSION``
since the last tag counts as a breaking change, because older peers can no
longer connect.

Writing updates ``[workspace.package] version`` in ``Cargo.toml``, refreshes
``Cargo.lock``, and regenerates ``CHANGELOG.md`` with git-cliff. Committing and
tagging are left to the caller. Running it again before the release is tagged
recomputes the version from the last tag and refreshes the changelog.
"""

import argparse
import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CARGO_TOML = ROOT / "Cargo.toml"
CHANGELOG = ROOT / "CHANGELOG.md"
PROTOCOL_FILE = "crates/egui-states/src/lib.rs"

LEVELS = ("patch", "minor", "major")
VERSION_RE = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")
SUBJECT_RE = re.compile(r"^(?P<type>[a-zA-Z]+)(?:\([^)]*\))?(?P<breaking>!)?:")
FOOTER_RE = re.compile(r"^BREAKING[ -]CHANGE:", re.MULTILINE)
PROTOCOL_RE = re.compile(r"PROTOCOL_VERSION:\s*u16\s*=\s*(\d+)\s*;")
WORKSPACE_VERSION_RE = re.compile(
    r'(\[workspace\.package\][^\[]*?^version\s*=\s*")([^"]+)(")',
    re.MULTILINE | re.DOTALL,
)


class ReleaseError(Exception):
    """A condition that stops the release from being prepared."""


def git(*args: str) -> str:
    """Runs git in the repository root and returns its stripped output.

    Returns:
        str: The command's standard output without surrounding whitespace.
    """
    return subprocess.run(["git", *args], cwd=ROOT, check=True, capture_output=True, text=True).stdout.strip()


def parse_version(text: str) -> tuple[int, int, int]:
    """Parses a plain ``MAJOR.MINOR.PATCH`` version.

    Returns:
        tuple[int, int, int]: The three version components.

    Raises:
        ReleaseError: If the text is not a plain version.
    """
    match = VERSION_RE.match(text)
    if match is None:
        raise ReleaseError(f"{text!r} is not a MAJOR.MINOR.PATCH version")
    major, minor, patch = (int(part) for part in match.groups())
    return major, minor, patch


def bump(version: tuple[int, int, int], level: str) -> str:
    """Applies a bump level to a version.

    Returns:
        str: The bumped version.
    """
    major, minor, patch = version
    if level == "major":
        return f"{major + 1}.0.0"
    if level == "minor":
        return f"{major}.{minor + 1}.0"
    return f"{major}.{minor}.{patch + 1}"


def breaking_level(version: tuple[int, int, int]) -> str:
    """Returns the level a breaking change needs: major, or minor before 1.0.

    Returns:
        str: ``"minor"`` for 0.x versions, ``"major"`` otherwise.
    """
    return "minor" if version[0] == 0 else "major"


def last_tag() -> str:
    """Finds the nearest version tag reachable from HEAD.

    Returns:
        str: The tag name.

    Raises:
        ReleaseError: If no version tag is reachable.
    """
    try:
        tag = git("describe", "--tags", "--abbrev=0", "--match", "[0-9]*.[0-9]*.[0-9]*", "HEAD")
    except subprocess.CalledProcessError as error:
        raise ReleaseError("no version tag is reachable from HEAD; fetch the tags first") from error
    parse_version(tag)
    return tag


def commit_level(tag: str, version: tuple[int, int, int]) -> tuple[str, bool, list[str]]:
    """Derives the bump level from the commits since the tag.

    Returns:
        tuple[str, bool, list[str]]: The level, whether a commit is breaking,
        and the commit titles that decided the level.

    Raises:
        ReleaseError: If there are no commits since the tag.
    """
    log = git("log", "--no-merges", "--format=%s%x1f%b%x1e", f"{tag}..HEAD")
    # The separators count as whitespace for str.strip, so an empty body can
    # lose its separator; partition copes with a missing one.
    commits = [entry.strip("\n").partition("\x1f") for entry in log.split("\x1e") if entry.strip()]
    if not commits:
        raise ReleaseError(f"there are no commits since {tag}")

    breaking, features = [], []
    for subject, _, body in commits:
        match = SUBJECT_RE.match(subject)
        if (match and match["breaking"]) or FOOTER_RE.search(body):
            breaking.append(subject)
        elif match and match["type"] == "feat":
            features.append(subject)
    if breaking:
        return breaking_level(version), True, breaking
    if features:
        return "minor", False, features
    return "patch", False, []


def protocol_version(source: str) -> int:
    """Reads ``PROTOCOL_VERSION`` from the library source.

    Returns:
        int: The protocol version.

    Raises:
        ReleaseError: If the constant is missing.
    """
    match = PROTOCOL_RE.search(source)
    if match is None:
        raise ReleaseError(f"PROTOCOL_VERSION not found in {PROTOCOL_FILE}")
    return int(match[1])


def protocol_change(tag: str) -> tuple[int, int]:
    """Compares ``PROTOCOL_VERSION`` at the tag with the working tree.

    Returns:
        tuple[int, int]: The released and the current protocol version.

    Raises:
        ReleaseError: If the protocol version moved by anything but one.
    """
    released = protocol_version(git("show", f"{tag}:{PROTOCOL_FILE}"))
    current = protocol_version((ROOT / PROTOCOL_FILE).read_text(encoding="utf-8"))
    if current not in {released, released + 1}:
        raise ReleaseError(
            f"PROTOCOL_VERSION is {current} but {tag} released {released}; "
            f"it must stay {released} or become {released + 1}"
        )
    return released, current


def choose_level(requested: str, tag: str, version: tuple[int, int, int]) -> tuple[str, list[str]]:
    """Combines the requested bump with the commit and protocol checks.

    An explicit level may be smaller than the one ``feat`` commits suggest, but
    never smaller than a breaking change requires.

    Returns:
        tuple[str, list[str]]: The level and the reasons for it.

    Raises:
        ReleaseError: If the requested level is too small for a breaking change.
    """
    level, is_breaking, deciding = commit_level(tag, version)
    reasons = [f"{level} from commit: {subject}" for subject in deciding] or ["patch: no feat or breaking commits"]

    released, current = protocol_change(tag)
    if current != released:
        is_breaking = True
        level = max(level, breaking_level(version), key=LEVELS.index)
        reasons.append(f"{breaking_level(version)} required: PROTOCOL_VERSION changed {released} -> {current}")

    if requested == "auto":
        return level, reasons
    if is_breaking and LEVELS.index(requested) < LEVELS.index(breaking_level(version)):
        raise ReleaseError(f"a {requested} release cannot contain breaking changes:\n  " + "\n  ".join(reasons))
    return requested, [*reasons, f"{requested} requested"]


def write_cargo_version(new_version: str) -> None:
    """Sets ``[workspace.package] version`` in ``Cargo.toml``.

    Raises:
        ReleaseError: If the version line cannot be found.
    """
    text = CARGO_TOML.read_text(encoding="utf-8")
    updated, count = WORKSPACE_VERSION_RE.subn(rf"\g<1>{new_version}\g<3>", text, count=1)
    if count != 1:
        raise ReleaseError("could not find [workspace.package] version in Cargo.toml")
    CARGO_TOML.write_text(updated, encoding="utf-8")


def main() -> int:
    """Runs the command line interface.

    Returns:
        int: The process exit code.
    """
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--bump", choices=("auto", *LEVELS), default="auto", help="version bump (default: auto)")
    parser.add_argument("--dry-run", action="store_true", help="print the next version without writing files")
    parser.add_argument("--git-cliff", default="git-cliff", help="git-cliff executable (default: git-cliff)")
    args = parser.parse_args()

    try:
        tag = last_tag()
        version = parse_version(tag)
        cargo_version = tomllib.loads(CARGO_TOML.read_text(encoding="utf-8"))["workspace"]["package"]["version"]
        if parse_version(cargo_version) < version:
            raise ReleaseError(f"Cargo.toml has version {cargo_version}, older than the last tag {tag}")
        # A version ahead of the last tag is a prepared release that was never
        # tagged, for example because its changelog went stale. Preparing again
        # recomputes the version from the last tag and refreshes the changelog.
        if cargo_version != tag:
            print(f"refreshing the untagged release {cargo_version}")
        level, reasons = choose_level(args.bump, tag, version)
        new_version = bump(version, level)

        print(f"{tag} -> {new_version} ({level})")
        for reason in reasons:
            print(f"  {reason}")
        if args.dry_run:
            return 0

        if git("status", "--porcelain"):
            raise ReleaseError("the working tree has uncommitted changes")
        write_cargo_version(new_version)
        subprocess.run(["cargo", "update", "--workspace"], cwd=ROOT, check=True)
        # Pass the output explicitly: git-cliff truncates the configured output
        # file even for commands that print to stdout.
        subprocess.run([args.git_cliff, "--tag", new_version, "--output", str(CHANGELOG)], cwd=ROOT, check=True)
        if not git("status", "--porcelain"):
            raise ReleaseError(f"release {new_version} is already prepared and up to date; push its tag")
    except ReleaseError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    if output := os.environ.get("GITHUB_OUTPUT"):
        with open(output, "a", encoding="utf-8") as file:
            file.write(f"version={new_version}\nlevel={level}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
