"""Buffer contracts through the public Python wrappers and native storage."""

import numpy as np
import pytest
from .assertions import assert_array

DTYPES = [np.uint8, np.int8, np.uint16, np.int16, np.uint32, np.int32, np.uint64, np.int64, np.float32, np.float64]
NAMES = ["u8", "i8", "u16", "i16", "u32", "i32", "u64", "i64", "f32", "f64"]


@pytest.mark.parametrize("name,dtype", list(zip(NAMES, DTYPES)), ids=NAMES)
def test_numeric_storage_and_replacement(local_bundle, name, dtype):
    _, states, _ = local_bundle
    data = getattr(states.numeric, name + "_data")
    limits = np.iinfo(dtype) if np.issubdtype(dtype, np.integer) else None
    expected = np.array([limits.min, 0, limits.max] if limits else [-1.25, 0, 9.5], dtype=dtype)
    data.set(expected)
    assert_array(data.get(), expected)
    replacement = np.array([2, 3], dtype=dtype)
    data.set(replacement)
    assert_array(data.get(), replacement)
    data.clear()
    assert_array(data.get(), np.array([], dtype=dtype))


@pytest.mark.parametrize("operation", ["set", "add", "replace"])
@pytest.mark.parametrize("invalid", ["dtype", "noncontiguous", "byteorder"])
def test_rejected_numeric_buffers_preserve_state(local_bundle, operation, invalid):
    _, states, _ = local_bundle
    data = states.numeric.u16_data
    before = np.array([11, 22, 33], dtype=np.uint16)
    data.set(before)
    bad = {
        "dtype": np.array([9], dtype=np.float64),
        "noncontiguous": np.arange(6, dtype=np.uint16)[::2],
        "byteorder": np.array([9], dtype=np.dtype(np.uint16).newbyteorder("S")),
    }[invalid]
    error = None
    try:
        if operation == "replace":
            data.replace(bad, 0)
        else:
            getattr(data, operation)(bad)
    except (ValueError, BufferError) as caught:
        error = caught
    assert error is not None, (
        f"{operation} accepted {invalid} buffer {bad.dtype.str}: {bad.tolist()} became {data.get().tolist()}"
    )
    assert_array(data.get(), before)


@pytest.mark.parametrize("channels", [1, 2, 3, 4], ids=["gray", "gray-alpha", "rgb", "rgba"])
@pytest.mark.parametrize("padded", [False, True], ids=["contiguous", "padded-rows"])
def test_image_formats_and_padded_rows(local_bundle, channels, padded):
    _, states, _ = local_bundle
    shape = (3, 7 if padded else 4) + (() if channels == 1 else (channels,))
    storage = np.arange(np.prod(shape), dtype=np.uint8).reshape(shape)
    data = storage[:, :4]
    rgba = np.empty((3, 4, 4), dtype=np.uint8)
    if channels <= 2:
        gray = data if channels == 1 else data[..., 0]
        rgba[..., :3] = gray[..., None]
    else:
        rgba[..., :3] = data[..., :3]
    rgba[..., 3] = data[..., -1] if channels in (2, 4) else 255
    states.image.image.set(data)
    assert_array(states.image.image.get(), rgba)
    states.image.images[79].set(data)
    assert_array(states.image.images[79].get(), rgba)
    patch = data[:2, :2]
    states.image.image.update(patch, (1, 1))
    expected = rgba.copy()
    expected[1:3, 1:3] = rgba[:2, :2]
    assert_array(states.image.image.get(), expected)


@pytest.mark.parametrize("view", ["rows", "columns", "channels"])
def test_invalid_image_strides_preserve_image(local_bundle, view):
    _, states, _ = local_bundle
    image = np.arange(3 * 4 * 4, dtype=np.uint8).reshape(3, 4, 4)
    states.image.image.set(image)
    invalid = {"rows": image[::-1], "columns": image[:, ::2], "channels": image[..., ::-1]}[view]
    with pytest.raises(ValueError, match="strides"):
        states.image.image.set(invalid)
    assert_array(states.image.image.get(), image)
