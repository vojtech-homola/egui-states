"""Collection behavior independent of showcase action callbacks."""


def test_vector_mutation(local_bundle):
    """Verify vector replacement, append, removal, and clearing."""
    _, states, _ = local_bundle
    values = states.value_vec.items
    values.set([10, -3, 27], update=True)
    assert values.get() == [10, -3, 27]
    values.add_item(32, update=True)
    assert values.get() == [10, -3, 27, 32]
    values.remove_item(3, update=True)
    assert values.get() == [10, -3, 27]
    values.set([], update=True)
    assert values.get() == []


def test_map_mutation(local_bundle):
    """Verify map replacement, insertion, removal, and clearing."""
    _, states, _ = local_bundle
    values = states.value_map.items
    values.set({1: 100, 2: 200, 5: 500}, update=True)
    assert values.get() == {1: 100, 2: 200, 5: 500}
    values.set_item(6, 600, update=True)
    assert values.get() == {1: 100, 2: 200, 5: 500, 6: 600}
    values.remove_item(1, update=True)
    assert values.get() == {2: 200, 5: 500, 6: 600}
    values.set({}, update=True)
    assert values.get() == {}


def test_vector_boundary_errors_preserve_state(local_bundle):
    import pytest

    _, states, _ = local_bundle
    values = states.value_vec.items
    for before in ([], [10], [10, 20, 30]):
        values.set(before)
        for bad in (-1, len(before), len(before) + 100):
            with pytest.raises((ValueError, OverflowError)):
                values.remove_item(bad)
            assert values.get() == before
    values.remove_item(0)
    assert values.get() == [20, 30]
    values.remove_item(1)
    assert values.get() == [20]


def test_collection_operation_sequences_match_reference_models(local_bundle):
    import random
    import pytest

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
                expected_vector.pop(index)
                vector.remove_item(index)
            if key in expected_map:
                del expected_map[key]
                mapping.remove_item(key)
            else:
                with pytest.raises(ValueError, match="Key not found"):
                    mapping.remove_item(key)
        else:
            expected_vector.append(value)
            vector.add_item(value)
            expected_map[key] = value
            mapping.set_item(key, value)
        assert vector.get() == expected_vector, f"vector after operation {step}"
        assert mapping.get() == expected_map, f"map after operation {step}"


def test_sparse_numeric_reset_and_invalid_mutations(local_bundle):
    import numpy as np
    import pytest
    from .assertions import assert_array

    _, states, _ = local_bundle
    data = states.multi_data.bytes
    for key in [0, 1, 2**32 - 1]:
        data[key].set(np.array([1, 2, 3], dtype=np.uint8))
    for operation in ("replace", "remove"):
        with pytest.raises(ValueError):
            if operation == "replace":
                data[1].replace(np.array([9], dtype=np.uint8), 4)
            else:
                data[1].remove(2, 2)
        assert_array(data[1].get(), np.array([1, 2, 3], dtype=np.uint8))
    data.remove_index(777)  # missing removal is harmless
    data.reset()
    for key in [0, 1, 2**32 - 1]:
        with pytest.raises(ValueError, match="index not found"):
            data[key].get()


def test_collection_overwrites_and_rejected_items(local_bundle):
    import pytest

    _, states, _ = local_bundle
    vector, mapping = states.value_vec.items, states.value_map.items
    vector.set([10, 20, 30])
    vector.set_item(0, -1)
    vector.set_item(2, -3)
    assert vector.get() == [-1, 20, -3]
    for index in [-1, 3]:
        with pytest.raises((ValueError, OverflowError)):
            vector.set_item(index, 99)
        assert vector.get() == [-1, 20, -3]
    mapping.set({7: 17})
    mapping.set_item(7, 71)
    assert mapping.get() == {7: 71}
    with pytest.raises(ValueError, match="Key not found"):
        mapping.get_item(8)
    for key, value in [(2**16, 1), (8, -1), (8, "wrong")]:
        with pytest.raises((ValueError, OverflowError, TypeError)):
            mapping.set_item(key, value)
        assert mapping.get() == {7: 71}
