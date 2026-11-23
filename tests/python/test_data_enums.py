"""Enums with data: generated variants and their conversion, without a client."""

import copy
import dataclasses
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
    Command.Stop(),
    Command.MoveTo(1.5, -2.5),
    Command.SetGain(0.25),
    Command.Pair(-3, "pair"),
    Command.Choose(Choice.C),
    Command.Load("a.tif", Summary(True, 7, "seven")),
    Command.Load("b.tif", None),
    Command.Report(Status.Busy(0.5)),
    Command.Report(Status.Idle()),
    Command.None_(),
    Command.Keyword(255),
]


def describe(command: Command.Type) -> str:
    """Names the variant, reading its fields in a ``match``."""
    match command:
        case Command.Stop() | Command.None_():
            return "unit"
        case Command.MoveTo(x, y):
            return f"move {x} {y}"
        case Command.SetGain(gain):
            return f"gain {gain}"
        case Command.Pair(number, text):
            return f"pair {number} {text}"
        case Command.Choose(choice):
            return f"choose {choice.name}"
        case Command.Load(path, None):
            return f"load {path}"
        case Command.Load(path, summary):
            return f"load {path} {summary.name if summary else None}"
        case Command.Report(Status.Busy(progress)):
            return f"busy {progress}"
        case Command.Report(status):
            return f"report {status}"
        case Command.Keyword(from_=value):
            return f"keyword {value}"
        case _:
            # Not `assert_never`: type checkers do not see the generated bindings here.
            raise AssertionError(command)


def test_every_variant_is_a_frozen_dataclass():
    """Unit, named and tuple variants are the same kind of class."""
    for command in ALL_COMMANDS:
        assert dataclasses.is_dataclass(command)
        assert not hasattr(command, "__dict__")
        assert copy.deepcopy(command) == command
        assert pickle.loads(pickle.dumps(command)) == command
        assert copy.copy(command) == command
    assert [describe(command) for command in ALL_COMMANDS] == [
        "unit",
        "move 1.5 -2.5",
        "gain 0.25",
        "pair -3 pair",
        "choose C",
        "load a.tif seven",
        "load b.tif",
        "busy 0.5",
        "report TestStatus.Idle()",
        "unit",
        "keyword 255",
    ]


def test_the_enum_is_a_namespace_of_its_variants():
    """The enum class has no values; its ``Type`` is the union of the variants."""
    with pytest.raises(TypeError, match="TestCommand is a namespace of variants"):
        Command()
    variants = Command.Type.__value__
    assert all(isinstance(command, variants) for command in ALL_COMMANDS)
    assert not isinstance(Status.Idle(), variants)
    assert not isinstance(Command.Stop, variants)


def test_unit_variants_are_created_like_the_others():
    """A unit variant is a class without fields, so its values are equal."""
    assert repr(Command.Stop()) == "TestCommand.Stop()"
    assert repr(Command.None_()) == "TestCommand.None_()"
    assert isinstance(Command.Stop(), Command.Stop)
    assert Command.Stop() == Command.Stop()
    assert Command.Stop() != Command.None_()
    assert Command.Stop() != Status.Idle()
    assert hash(Command.Stop()) == hash(Command.Stop())
    with pytest.raises(TypeError):
        Command.Stop(1)
    match Command.Stop():
        case Command.None_():
            pytest.fail("matched the wrong variant")
        case Command.Stop():
            pass
        case _:
            pytest.fail("no match")


def test_named_variants_have_named_fields():
    """A variant with named fields is created and matched by name or position."""
    move = Command.MoveTo(x=1.0, y=2.0)
    assert repr(move) == "TestCommand.MoveTo(x=1.0, y=2.0)"
    assert move == Command.MoveTo(1.0, 2.0)
    assert move != Command.MoveTo(1.0, 3.0)
    assert isinstance(move, Command.MoveTo)
    assert (move.x, move.y) == (1.0, 2.0)
    with pytest.raises(AttributeError):
        move.x = 3.0
    assert dataclasses.replace(move, y=5.0) == Command.MoveTo(1.0, 5.0)
    match move:
        case Command.MoveTo(x=x, y=y):
            assert (x, y) == (1.0, 2.0)
        case _:
            pytest.fail("no match")
    assert Command.Keyword(from_=3).from_ == 3


def test_tuple_variants_have_positional_fields():
    """A tuple variant is created and matched by position, and is not a tuple."""
    pair = Command.Pair(1, "x")
    assert repr(pair) == "TestCommand.Pair(1, 'x')"
    assert repr(Command.Report(Status.Failed("f"))) == "TestCommand.Report(TestStatus.Failed('f'))"
    assert pair == Command.Pair(1, "x")
    assert pair != Command.Pair(1, "y")
    assert pair != (1, "x")
    assert Command.SetGain(1.0) != Command.Choose(1.0)
    assert not isinstance(pair, tuple)
    assert {pair: 1}[Command.Pair(1, "x")] == 1
    # A variant is hashable when its fields are; a struct is mutable, so it is not.
    with pytest.raises(TypeError, match="unhashable"):
        hash(Command.Load("a.tif", Summary(True, 7, "seven")))
    match pair:
        case Command.Pair(a, b):
            assert (a, b) == (1, "x")
        case _:
            pytest.fail("no match")
    # None of what a tuple adds: no length, index, unpacking or ordering.
    with pytest.raises(TypeError):
        len(pair)
    with pytest.raises(TypeError):
        pair[0]
    with pytest.raises(TypeError):
        _, _ = pair
    with pytest.raises(TypeError):
        _ = pair < Command.Pair(2, "x")
    with pytest.raises(TypeError):
        Command.Pair(1)
    with pytest.raises(AttributeError):
        pair._0 = 2


@pytest.mark.parametrize("command", ALL_COMMANDS, ids=repr)
def test_every_variant_round_trips(local_bundle, command):
    """Every variant shape is encoded and decoded back to its own class."""
    _, states, _ = local_bundle
    handle = states.data_enums.command
    handle.set(command)
    back = handle.get()
    assert back == command
    assert type(back) is type(command)


def test_a_variant_class_is_not_a_value(local_bundle):
    """Passing the class of a variant instead of a value names the fix."""
    _, states, _ = local_bundle
    handle = states.data_enums.command
    with pytest.raises(TypeError, match=r"got the class TestCommand\.Stop; create a value with TestCommand\.Stop\(\)"):
        handle.set(Command.Stop)
    with pytest.raises(TypeError, match=r"create a value with TestCommand\.MoveTo\(\.\.\.\)"):
        handle.set(Command.MoveTo)
    with pytest.raises(TypeError, match="Expected a variant of TestCommand, got TestStatus.Idle()"):
        handle.set(Status.Idle())


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
    e.fixed.set(Command.Report(Status.Idle()))
    assert e.fixed.get() == Command.Report(Status.Idle())
    e.list.set(ALL_COMMANDS)
    assert e.list.get() == ALL_COMMANDS
    e.table.set({1: Status.Idle(), 2: Status.Busy(1.0)})
    assert e.table.get() == {1: Status.Idle(), 2: Status.Busy(1.0)}


@pytest.mark.parametrize(
    "invalid",
    [
        (1.5, -2.0),
        (-3, "pair"),
        Choice.A,
        Status.Idle(),
        Command.Stop,
        "MoveTo",
        None,
        Command.Pair(1, 2),
        Command.MoveTo("x", 1.0),
        Command.Report(Command.Stop()),
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


@pytest.mark.parametrize(
    ("invalid", "error", "message"),
    [
        (Command.SetGain("text"), TypeError, "TestCommand.SetGain._0: must be real number, not str"),
        (Command.Pair(1, 2), TypeError, "TestCommand.Pair._1: "),
        (Command.Keyword(300), OverflowError, "TestCommand.Keyword.from_: "),
        (Command.Choose(1), TypeError, "TestCommand.Choose._0: Expected a member of TestEnum, got 1"),
        (Command.Load("a", "x"), TypeError, "TestCommand.Load.summary: Expected an instance of TestStruct2, got 'x'"),
        (Command.Load("a", Summary(True, "high", "n")), TypeError, "TestCommand.Load.summary.level: "),
        (
            Command.Report(Command.Stop()),
            TypeError,
            "TestCommand.Report._0: Expected a variant of TestStatus, got TestCommand.Stop()",
        ),
        (Command.Report(Status.Busy("half")), TypeError, "TestCommand.Report._0.Busy.progress: must be real number"),
    ],
    ids=repr,
)
def test_an_invalid_field_is_named_in_the_error(local_bundle, invalid, error, message):
    """The error for a wrong field starts with the path to that field."""
    _, states, _ = local_bundle
    with pytest.raises(error) as raised:
        states.data_enums.command.set(invalid)
    assert str(raised.value).startswith(message)


def test_an_invalid_field_is_named_inside_structs_and_collections(local_bundle):
    """The path covers struct fields, list items and map keys and values."""
    _, states, _ = local_bundle
    e = states.data_enums
    with pytest.raises(TypeError, match=r"^TestJob\.status\.Busy\.progress: must be real number"):
        e.job.set(Job("job", Status.Busy("half")))
    with pytest.raises(TypeError, match=r"^TestJob\.status: Expected a variant of TestStatus, got 3$"):
        e.job.set(Job("job", 3))

    with pytest.raises(TypeError, match=r"^\[1\]\.SetGain\._0: must be real number"):
        e.list.set([Command.Stop(), Command.SetGain("text")])
    assert e.list.get() == []
    with pytest.raises(TypeError, match=r"^TestCommand\.SetGain\._0: must be real number"):
        e.list.add_item(Command.SetGain("text"))

    with pytest.raises(TypeError, match=r"^\[2\]\.Busy\.progress: must be real number"):
        e.table.set({1: Status.Idle(), 2: Status.Busy("half")})
    with pytest.raises(TypeError, match=r"^\[key 1\.5\]: "):
        e.table.set({1.5: Status.Idle()})
    with pytest.raises(TypeError, match=r"^\[key 'one'\]: "):
        e.table["one"] = Status.Idle()
    assert e.table.get() == {}


def test_an_error_that_is_not_one_message_gets_the_path_as_a_note(local_bundle):
    """An exception with several arguments keeps them; the path is a note."""
    _, states, _ = local_bundle
    with pytest.raises(UnicodeEncodeError) as raised:
        states.data_enums.command.set(Command.Load("\ud800", None))
    assert raised.value.__notes__ == ["In TestCommand.Load.path"]


def test_a_wrong_value_itself_has_no_path(local_bundle):
    """A value of the wrong type at the top is reported as before."""
    _, states, _ = local_bundle
    with pytest.raises(TypeError, match=r"^Expected an instance of TestJob, got 'job'$"):
        states.data_enums.job.set("job")
    with pytest.raises(TypeError, match=r"^Expected a variant of TestCommand, got 1\.5$"):
        states.data_enums.command.set(1.5)


def test_enums_with_data_as_headers(local_bundle):
    """Enums with data as Data and image headers."""
    _, states, _ = local_bundle
    e = states.data_enums
    header = Command.Load("frame.tif", None)
    e.data.set(np.array([1, 2], dtype=np.uint8), header=header)
    assert e.data.get_header() == header
    e.data.set_header(Command.Stop())
    assert e.data.get_header() == Command.Stop()

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
    e.data_multi_take[1].set(np.array([6], dtype=np.uint8), header=Command.None_())
    with pytest.raises(TypeError):
        e.data.set_header(Status.Idle())
