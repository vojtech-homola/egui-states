docker run --rm -v "$PWD:/io" -w /io quay.io/pypa/manylinux_2_28_x86_64 bash -lc '
  set -eux
  export CARGO_TARGET_DIR=/tmp/cargo-target

  curl -LsSf https://astral.sh/uv/install.sh | sh
  export PATH="$HOME/.local/bin:$PATH"

  uv build --wheel -p 3.12
  uv build --wheel -p 3.13
  uv build --wheel -p 3.14
  uv build --wheel -p 3.14t
'
