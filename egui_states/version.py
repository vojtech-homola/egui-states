"""Package version, compiled into the native extension from Cargo.toml."""

from egui_states._core import __version__

VERSION = __version__

__all__ = ["VERSION", "__version__"]
