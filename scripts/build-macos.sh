set -eux

# Apple silicon only.
rustup target add aarch64-apple-darwin

uv build --wheel -p 3.12 -C maturin.build-args="--target aarch64-apple-darwin --compression-level 9"
uv build --wheel -p 3.13 -C maturin.build-args="--target aarch64-apple-darwin --compression-level 9"
uv build --wheel -p 3.14 -C maturin.build-args="--target aarch64-apple-darwin --compression-level 9"
uv build --wheel -p 3.14t -C maturin.build-args="--target aarch64-apple-darwin --compression-level 9"
uv build --wheel -p 3.15 -C maturin.build-args="--target aarch64-apple-darwin --compression-level 9"
uv build --wheel -p 3.15t -C maturin.build-args="--target aarch64-apple-darwin --compression-level 9"
