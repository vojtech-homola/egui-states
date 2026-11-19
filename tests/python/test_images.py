# ruff: noqa: D103
"""Image conversion, updates, and validation through generated state handles."""

import numpy as np
import pytest
from .assertions import assert_array

from egui_states_test_bindings import (  # ty: ignore[import-not-found]
    State,
    StatesServer,
)


@pytest.mark.parametrize("multi", [False, True], ids=["image", "image-multi"])
@pytest.mark.parametrize("shape", [(65_536, 1), (1, 65_536)], ids=["height-limit", "width-limit"])
def test_image_shape_limit(local_bundle, multi, shape):
    _, states, _ = local_bundle
    image = states.image.images[7] if multi else states.image.image
    pixels = np.full(shape, 7, dtype=np.uint8)
    image.set(pixels)
    assert image.shape() == shape
    assert_array(image.get(), pixels)
    image.set_all(shape, 9)
    origin = (shape[0] - 1, shape[1] - 1)
    image.update(np.array([[8]], dtype=np.uint8), origin)
    assert image.get()[origin] == 8
    before = image.get().copy()

    for oversized in [(65_537, 1), (1, 65_537)]:
        pixels = np.zeros(oversized, dtype=np.uint8)
        with pytest.raises(ValueError, match="65536"):
            image.set(pixels)
        with pytest.raises(ValueError, match="65536"):
            image.set_all(oversized, 0)
        with pytest.raises(ValueError, match="65536"):
            image.update(pixels, (0, 0))
        assert_array(image.get(), before)
    for origin in [(65_536, 0), (0, 65_536)]:
        with pytest.raises(ValueError, match="65536"):
            image.update(np.zeros((1, 1), dtype=np.uint8), origin)
        assert_array(image.get(), before)
    if multi:
        with pytest.raises(ValueError, match="65536"):
            states.image.images[9].set_all((65_537, 1), 0)
        assert 9 not in states.image.images


def test_image_value_roundtrip(local_bundle: tuple[StatesServer, State, list[Exception]]) -> None:
    _server, states, _errors = local_bundle

    image = np.zeros((8, 8, 4), dtype=np.uint8)
    image[..., 0] = 10
    image[..., 3] = 255
    states.image.image.set(image)

    image_result = states.image.image.get()
    assert states.image.image.shape() == (8, 8)
    assert_array(image_result, image)

    patch = np.zeros((2, 3, 4), dtype=np.uint8)
    patch[..., 1] = 80
    patch[..., 3] = 200
    states.image.image.update(patch, origin=(3, 2))

    image_result = states.image.image.get()
    expected = image.copy()
    expected[3:5, 2:5] = patch
    assert_array(image_result, expected)
    assert states.image.image.shape() == (8, 8)


def test_image_fill_colors(local_bundle):
    _, states, _ = local_bundle
    # The image is stored in the format of the color.
    assert_array(_fill(states.image.image, 7), np.full((2, 3), 7, dtype=np.uint8))
    for color in [(7, 8), (7, 8, 9), (7, 8, 9, 10)]:
        expected = np.tile(np.array(color, dtype=np.uint8), (2, 3, 1))
        assert_array(_fill(states.image.image, color), expected)


def _fill(image, color):
    image.set_all((2, 3), color)
    return image.get()


def test_image_is_stored_in_its_format_and_widened_by_updates(local_bundle):
    _, states, _ = local_bundle
    image = states.image.image
    assert image.get() is None
    assert image.shape() == (0, 0)

    gray = np.arange(6, dtype=np.uint8).reshape((2, 3))
    image.set(gray)
    assert_array(image.get(), gray)

    # Fewer channels than stored: converted while written, format kept.
    image.update(np.full((1, 1), 9, dtype=np.uint8), (0, 0))
    gray[0, 0] = 9
    assert_array(image.get(), gray)

    # More channels: the whole image is widened first.
    image.update(np.array([[[1, 2, 3]]], dtype=np.uint8), (1, 2))
    rgb = np.repeat(gray[..., None], 3, axis=-1)
    rgb[1, 2] = [1, 2, 3]
    assert_array(image.get(), rgb)

    # Gray-alpha and RGB meet at RGBA.
    image.update(np.array([[[5, 6]]], dtype=np.uint8), (0, 1))
    rgba = np.concatenate([rgb, np.full((2, 3, 1), 255, dtype=np.uint8)], axis=-1)
    rgba[0, 1] = [5, 5, 5, 6]
    assert_array(image.get(), rgba)

    # Only a set or fill narrows it again.
    image.set(gray)
    assert_array(image.get(), gray)

    images = states.image.images
    images[1].set(gray)
    images[2].set_all((1, 1), (1, 2, 3))
    assert_array(images[1].get(), gray)
    assert_array(images[2].get(), np.array([[[1, 2, 3]]], dtype=np.uint8))


def test_invalid_image_fill_preserves_pixels(local_bundle):
    _, states, _ = local_bundle
    states.image.image.set_all((2, 3), (7, 8, 9, 10))
    before = states.image.image.get().copy()
    with pytest.raises(ValueError, match="color must be"):
        states.image.image.set_all((2, 3), 256)
    with pytest.raises(ValueError, match="color must be"):
        states.image.image.set_all((2, 3), (1,))
    with pytest.raises(ValueError, match="color must be"):
        states.image.image.set_all((2, 3), [1, 2, 3])
    with pytest.raises(ValueError, match="dimensions cannot be zero"):
        states.image.image.set_all((0, 3), 1)
    assert_array(states.image.image.get(), before)


def test_image_multi_sparse_collection(local_bundle: tuple[StatesServer, State, list[Exception]]) -> None:
    _server, states, _errors = local_bundle
    images = states.image.images

    assert len(images) == 0
    assert images.indices() == []
    assert 7 not in images
    with pytest.raises(ValueError, match="ImageMulti index not found"):
        images[7].get()
    with pytest.raises(ValueError, match="ImageMulti index not found"):
        images[7].update(np.zeros((1, 1), dtype=np.uint8), (0, 0))

    images[7].set_all((3, 4), (1, 2, 3, 4))
    gray = np.arange(6, dtype=np.uint8).reshape((2, 3))
    images[2].set(gray)
    assert len(images) == 2
    assert images.indices() == [2, 7]
    assert 2 in images
    assert images[7].shape() == (3, 4)
    assert_array(
        images[7].get(),
        np.tile(np.array([1, 2, 3, 4], dtype=np.uint8), (3, 4, 1)),
    )

    patch = np.full((1, 2, 4), [20, 30, 40, 50], dtype=np.uint8)
    images[7].update(patch, origin=(1, 1))
    assert_array(images[7].get()[1:2, 1:3], patch)

    images[7].set_all((1, 2), 9)
    assert len(images) == 2
    assert images[7].shape() == (1, 2)

    with pytest.raises(BufferError):
        images[9].set(np.zeros((1, 1), dtype=np.int16))
    assert 9 not in images
    with pytest.raises(OverflowError):
        images[-1].set_all((1, 1), 0)
    with pytest.raises(ValueError, match="dimensions cannot be zero"):
        images[9].set(np.zeros((0, 1), dtype=np.uint8))
    with pytest.raises(ValueError, match="dimensions cannot be zero"):
        images[9].set_all((1, 0), 0)
    with pytest.raises(ValueError, match="dimensions cannot be zero"):
        images[7].update(np.zeros((1, 0), dtype=np.uint8), (0, 0))
    assert 9 not in images

    images.remove_index(100)
    assert len(images) == 2
    images.remove_index(2)
    assert images.indices() == [7]
    images.reset()
    assert len(images) == 0
    assert images.indices() == []


def test_image_update_outside_the_image_preserves_pixels(local_bundle):
    _, states, _ = local_bundle
    image = states.image.image
    with pytest.raises(ValueError):
        image.update(np.zeros((1, 1), dtype=np.uint8), (0, 0))
    image.set(np.arange(6, dtype=np.uint8).reshape(2, 3))
    before = image.get().copy()
    for origin, shape in [
        ((0, 0), (3, 1)),
        ((0, 0), (1, 4)),
        ((2, 0), (1, 1)),
        ((0, 3), (1, 1)),
        ((1, 2), (1, 2)),
        ((2**64 - 1, 0), (1, 1)),
    ]:
        with pytest.raises((ValueError, OverflowError)):
            image.update(np.zeros(shape, dtype=np.uint8), origin)
        assert_array(image.get(), before)
    image.update(np.full((1, 1), 9, dtype=np.uint8), (1, 2))
    assert image.get()[1, 2] == 9


@pytest.mark.parametrize("multi", [False, True], ids=["image", "image-multi"])
def test_image_wait_force_and_max_lines(local_bundle, multi):
    _, states, _ = local_bundle
    image = states.image.images[3] if multi else states.image.image
    state = states.image.images if multi else states.image.image
    # Without a client every change is only stored, so nothing waits.
    assert state.is_idle()
    image.set(np.zeros((4, 2), dtype=np.uint8), wait=True)
    assert_array(image.get(), np.zeros((4, 2), dtype=np.uint8))
    image.set_all((4, 2), 0, wait=True)

    rows = np.arange(8, dtype=np.uint8).reshape(4, 2)
    image.update(rows, (0, 0), max_lines=1)
    assert_array(image.get(), rows)
    image.update(np.full((2, 2), 9, dtype=np.uint8), (1, 0), force=True, max_lines=10, wait=True)
    assert state.is_idle()
    expected = rows.copy()
    expected[1:3] = 9
    assert_array(image.get(), expected)

    before = image.get().copy()
    for max_lines in [0, -1]:
        with pytest.raises(ValueError, match="max_lines"):
            image.update(np.zeros((1, 1), dtype=np.uint8), (0, 0), max_lines=max_lines)
    assert_array(image.get(), before)
