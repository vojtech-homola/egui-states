"""Array assertions shared by numeric and image tests."""

import numpy as np


def assert_array(actual, expected):
    assert actual.dtype == expected.dtype
    assert actual.shape == expected.shape
    np.testing.assert_array_equal(actual, expected)
