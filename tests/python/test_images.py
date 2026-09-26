# ruff: noqa: D103

import numpy as np
import pytest
from .assertions import assert_array

from egui_states_test_bindings import (
    State,
    StatesServer,
)


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
    states.image.image.update(patch, origin=(3, 2), update=True)

    image_result = states.image.image.get()
    expected = image.copy()
    expected[3:5, 2:5] = patch
    assert_array(image_result, expected)
    assert states.image.image.shape() == (8, 8)


def test_image_fill_colors(local_bundle):
    _, states, _ = local_bundle
    colors = (
        (7, [7, 7, 7, 255]),
        ((7, 8), [7, 7, 7, 8]),
        ((7, 8, 9), [7, 8, 9, 255]),
        ((7, 8, 9, 10), [7, 8, 9, 10]),
    )
    for color, expected in colors:
        states.image.image.set_all((2, 3), color, update=True)
        expected_image = np.tile(np.array(expected, dtype=np.uint8), (2, 3, 1))
        assert_array(states.image.image.get(), expected_image)


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
    images[7].update(patch, origin=(1, 1), update=True)
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
    images.remove_index(2, update=True)
    assert images.indices() == [7]
    images.reset(update=True)
    assert len(images) == 0
    assert images.indices() == []
