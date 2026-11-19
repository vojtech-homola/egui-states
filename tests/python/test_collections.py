"""Collection behavior independent of showcase action callbacks."""

import random

import pytest


def test_vector_mutation(local_bundle):
    """Verify vector replacement, append, removal, and clearing."""
    _, states, _ = local_bundle
    values = states.value_vec.items
    values.set([10, -3, 27])
    assert values.get() == [10, -3, 27]
    values.add_item(32)
    assert values.get() == [10, -3, 27, 32]
    assert values.remove_item(3) == 32
    assert values.get() == [10, -3, 27]
    values.set([])
    assert values.get() == []


def test_map_mutation(local_bundle):
    """Verify map replacement, insertion, removal, and clearing."""
    _, states, _ = local_bundle
    values = states.value_map.items
    values.set({1: 100, 2: 200, 5: 500})
    assert values.get() == {1: 100, 2: 200, 5: 500}
    values.set_item(6, 600)
    assert values.get() == {1: 100, 2: 200, 5: 500, 6: 600}
    assert values.remove_item(1) == 100
    assert values.get() == {2: 200, 5: 500, 6: 600}
    values.set({})
    assert values.get() == {}


def test_vector_boundary_errors_preserve_state(local_bundle):
    """Reject invalid vector indices without mutating stored items."""
    _, states, _ = local_bundle
    values = states.value_vec.items
    for before in ([], [10], [10, 20, 30]):
        values.set(before)
        for bad in (-len(before) - 1, len(before), len(before) + 100):
            with pytest.raises(IndexError):
                values.remove_item(bad)
            with pytest.raises(IndexError):
                values[bad]
            assert values.get() == before
    values.remove_item(0)
    assert values.get() == [20, 30]
    values.remove_item(1)
    assert values.get() == [20]


def test_collection_operation_sequences_match_reference_models(local_bundle):
    """Compare mixed collection operations with Python reference containers."""
    _, states, _ = local_bundle
    vector, mapping = states.value_vec.items, states.value_map.items
    expected_vector, expected_map = [], {}
    randomizer = random.Random(815)
    for step in range(100):
        value = randomizer.randrange(0, 1000)
        key = randomizer.choice([0, 1, 65535, 73])
        if step % 11 == 0:
            expected_vector = [value]
            expected_map = {key: value}
            vector.set(expected_vector)
            mapping.set(expected_map)
        elif step % 3 == 0:
            if expected_vector:
                index = randomizer.randrange(len(expected_vector))
                assert vector.remove_item(index) == expected_vector.pop(index)
            if key in expected_map:
                assert mapping.remove_item(key) == expected_map.pop(key)
            else:
                with pytest.raises(KeyError):
                    mapping.remove_item(key)
        else:
            expected_vector.append(value)
            vector.add_item(value)
            expected_map[key] = value
            mapping.set_item(key, value)
        assert vector.get() == expected_vector, f"vector after operation {step}"
        assert mapping.get() == expected_map, f"map after operation {step}"


def test_collection_overwrites_and_rejected_items(local_bundle):
    """Check replacement and invalid writes through vector and map handles."""
    _, states, _ = local_bundle
    vector, mapping = states.value_vec.items, states.value_map.items
    vector.set([10, 20, 30])
    vector.set_item(0, -1)
    vector.set_item(2, -3)
    assert vector.get() == [-1, 20, -3]
    for index in [-4, 3]:
        with pytest.raises(IndexError):
            vector.set_item(index, 99)
        assert vector.get() == [-1, 20, -3]
    mapping.set({7: 17})
    mapping.set_item(7, 71)
    assert mapping.get() == {7: 71}
    with pytest.raises(KeyError):
        mapping.get_item(8)
    for key, value in [(2**16, 1), (8, -1), (8, "wrong")]:
        with pytest.raises((ValueError, OverflowError, TypeError)):
            mapping.set_item(key, value)
        assert mapping.get() == {7: 71}


def test_vector_behaves_like_a_list(local_bundle):
    """Support len, iteration, membership, negative indices and deletion."""
    _, states, _ = local_bundle
    values = states.value_vec.items
    values.set([])
    assert len(values) == 0
    assert list(values) == []
    assert 1 not in values
    values.set([10, 20, 30])
    assert len(values) == 3  # noqa: PLR2004
    assert list(values) == [10, 20, 30]
    assert [value * 2 for value in values] == [20, 40, 60]
    assert 20 in values
    assert 25 not in values
    assert values[-1] == 30  # noqa: PLR2004
    assert values[-3] == 10  # noqa: PLR2004
    values[-1] = 31
    del values[0]
    assert values.get() == [20, 31]
    assert values.remove_item(-2) == 20  # noqa: PLR2004
    assert values.get() == [31]


def test_map_behaves_like_a_dict(local_bundle):
    """Support len, key iteration, membership and missing-key errors."""
    _, states, _ = local_bundle
    mapping = states.value_map.items
    mapping.set({})
    assert len(mapping) == 0
    assert list(mapping) == []
    assert 1 not in mapping
    mapping.set({1: 100, 2: 200})
    assert len(mapping) == 2  # noqa: PLR2004
    assert sorted(mapping) == [1, 2]
    assert 1 in mapping
    assert 3 not in mapping
    # A key of the wrong type is simply not in the map, as for a dict.
    assert "1" not in mapping
    assert 2**40 not in mapping
    with pytest.raises(KeyError) as missing:
        mapping[3]
    assert missing.value.args == (3,)
    with pytest.raises(KeyError):
        del mapping[3]
    del mapping[1]
    assert mapping.get() == {2: 200}
