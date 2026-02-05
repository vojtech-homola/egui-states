docker run --rm -v "$PWD:/io" -w /io quay.io/pypa/manylinux_2_28_x86_64 bash -lc '
  set -eux
  export CARGO_TARGET_DIR=/tmp/cargo-target

  curl -LsSf https://astral.sh/uv/install.sh | sh
  export PATH="$HOME/.local/bin:$PATH"

  uv build --wheel -p 3.12 -C maturin.build-args="--manylinux 2_28 --auditwheel repair --compression-level 9"
  uv build --wheel -p 3.13 -C maturin.build-args="--manylinux 2_28 --auditwheel repair --compression-level 9"
  uv build --wheel -p 3.14 -C maturin.build-args="--manylinux 2_28 --auditwheel repair --compression-level 9"
  uv build --wheel -p 3.14t -C maturin.build-args="--manylinux 2_28 --auditwheel repair --compression-level 9"
'