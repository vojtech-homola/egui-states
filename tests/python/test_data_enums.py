"""Enums with data: generated variants and their conversion, without a client."""

import copy
import pickle

import numpy as np
import pytest
from egui_states_test_bindings.enums import (  # ty: ignore[import-not-found]
    TestCommand as Command,
    TestEnum as Choice,
    TestStatus as Status,
)
from egui_states_test_bindings.structs import TestJob as Job, TestStruct2 as Summary  # ty: ignore[import-not-found]

from .assertions import assert_array

ALL_COMMANDS = [
    Command.Stop,
    Command.MoveTo(1.5, -2.5),
    Command.SetGain(0.25),
    Command.Pair(-3, "pair"),
    Command.Choose(Choice.C),
    Command.Load("a.tif", Summary(True, 7, "seven")),
    Command.Load("b.tif", None),
    Command.Report(Status.Busy(0.5)),
    Command.Report(Status.Idle),
    Command.None_,
    Command.Keyword(255),
]


def test_unit_variants_are_constants():
    """A unit variant is one constant that survives copies and pickling."""
    assert repr(Command.Stop) == "TestCommand.Stop"
    assert repr(Command.None_) == "TestCommand.None_"
    assert isinstance(Command.Stop, Command)
    assert Command.Stop == Command.Stop
    assert Command.Stop != Command.None_
    assert copy.copy(Command.Stop) is Command.Stop
    assert copy.deepcopy(Command.Stop) is Command.Stop
    assert pickle.loads(pickle.dumps(Command.Stop)) is Command.Stop
    match Command.Stop:
        case Command.None_:
            pytest.fail("matched the wrong constant")
        case Command.Stop:
            pass


def test_named_variants_are_frozen_dataclasses():
    """A variant with named fields is a frozen dataclass."""
    move = Command.MoveTo(x=1.0, y=2.0)
    assert repr(move) == "TestCommand.MoveTo(x=1.0, y=2.0)"
    assert move == Command.MoveTo(1.0, 2.0)
    assert move != Command.MoveTo(1.0, 3.0)
    assert isinstance(move, Command)
    assert isinstance(move, Command.MoveTo)
    with pytest.raises(AttributeError):
        move.x = 3.0
    match move:
        case Command.MoveTo(x, y):
            assert (x, y) == (1.0, 2.0)
        case _:
            pytest.fail("no match")
    assert Command.Keyword(from_=3).from_ == 3
    assert copy.deepcopy(move) == move
    assert pickle.loads(pickle.dumps(move)) == move


def test_tuple_variants_behave_like_tuples_of_their_variant():
    """A tuple variant indexes and unpacks like a tuple but equals only its own variant."""
    pair = Command.Pair(1, "x")
    assert repr(pair) == "TestCommand.Pair(1, 'x')"
    assert pair[0] == 1
    assert pair[1] == "x"
    assert len(pair) == 2
    first, second = pair
    assert (first, second) == (1, "x")
    assert pair == Command.Pair(1, "x")
    # Equality includes the variant: a plain tuple or another variant is not equal.
    assert pair != (1, "x")
    assert (1, "x") != pair
    assert Command.SetGain(1.0) != Command.Choose(1.0)
    assert hash(pair) != hash((1, "x"))
    assert {pair: 1}[Command.Pair(1, "x")] == 1
    match pair:
        case Command.Pair(a, b):
            assert (a, b) == (1, "x")
        case _:
            pytest.fail("no match")
    with pytest.raises(TypeError, match="takes 2 positional arguments"):
        Command.Pair(1)
    assert copy.deepcopy(pair) == pair
    assert pickle.loads(pickle.dumps(pair)) == pair


@pytest.mark.parametrize("command", ALL_COMMANDS, ids=repr)
def test_every_variant_round_trips(local_bundle, command):
    """Every variant shape is encoded and decoded back to its own class."""
    _, states, _ = local_bundle
    handle = states.data_enums.command
    handle.set(command)
    back = handle.get()
    assert back == command
    assert type(back) is type(command)


def test_unit_variants_come_back_as_their_constant(local_bundle):
    """A decoded unit variant is its constant, so ``is`` works."""
    _, states, _ = local_bundle
    states.data_enums.command.set(Command.None_)
    assert states.data_enums.command.get() is Command.None_


def test_initial_values_come_from_the_rust_definition(local_bundle):
    """The ``init`` of the Rust definition is the generated initial value."""
    _, states, _ = local_bundle
    e = states.data_enums
    assert e.command.get() == Command.MoveTo(1.5, -2.0)
    assert e.fixed.get() == Command.SetGain(0.5)
    assert e.job.get() == Job("job", Status.Busy(0.25))
    assert e.optional.get() == Status.Failed("init")


def test_enums_with_data_inside_other_types(local_bundle):
    """Enums with data inside structs, options, statics and collections."""
    _, states, _ = local_bundle
    e = states.data_enums
    e.job.set(Job("next", Status.Failed("broken")))
    assert e.job.get() == Job("next", Status.Failed("broken"))
    e.optional.set(None)
    assert e.optional.get() is None
    e.fixed.set(Command.Report(Status.Idle))
    assert e.fixed.get() == Command.Report(Status.Idle)
    e.list.set(ALL_COMMANDS)
    assert e.list.get() == ALL_COMMANDS
    e.table.set({1: Status.Idle, 2: Status.Busy(1.0)})
    assert e.table.get() == {1: Status.Idle, 2: Status.Busy(1.0)}


@pytest.mark.parametrize(
    "invalid",
    [
        (1.5, -2.0),
        Choice.A,
        Status.Idle,
        "MoveTo",
        None,
        Command.Pair(1, 2),
        Command.MoveTo("x", 1.0),
        Command.Report(Command.Stop),
    ],
    ids=repr,
)
def test_invalid_values_keep_the_previous_value(local_bundle, invalid):
    """A value that is not a valid variant is rejected."""
    _, states, _ = local_bundle
    handle = states.data_enums.command
    before = handle.get()
    with pytest.raises((TypeError, ValueError)):
        handle.set(invalid)
    assert handle.get() == before


def test_enums_with_data_as_headers(local_bundle):
    """Enums with data as Data and image headers."""
    _, states, _ = local_bundle
    e = states.data_enums
    header = Command.Load("frame.tif", None)
    e.data.set(np.array([1, 2], dtype=np.uint8), header=header)
    assert e.data.get_header() == header
    e.data.set_header(Command.Stop)
    assert e.data.get_header() is Command.Stop

    e.data_multi[4].set(np.array([3], dtype=np.uint8), header=Command.Pair(4, "four"))
    assert e.data_multi[4].get_header() == Command.Pair(4, "four")

    pixels = np.array([[1, 2]], dtype=np.uint8)
    e.image.set(pixels, header=Command.MoveTo(0.0, 1.0))
    image, image_header = e.image.get_with_header()
    assert_array(image, pixels)
    assert image_header == Command.MoveTo(0.0, 1.0)
    e.images[2].set(pixels, header=Command.SetGain(2.0))
    assert e.images[2].get_header() == Command.SetGain(2.0)

    # Without a client nothing is sent, but the header must still encode.
    e.data_take.set(np.array([5], dtype=np.uint8), header=Command.Choose(Choice.B))
    e.data_multi_take[1].set(np.array([6], dtype=np.uint8), header=Command.None_)
    with pytest.raises(TypeError):
        e.data.set_header(Status.Idle)
