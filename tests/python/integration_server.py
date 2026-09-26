"""Python server implementation for the shared native-client scenarios."""

import numpy as np
from egui_states_test_bindings import StatesServer
from tests.preparation import run
from tests.python_test_helpers.network import free_port


def configure(states):
    """Configure the test protocol; callback results are visible to the probe."""
    st = states.integration
    from egui_states_test_bindings.structs import WirePacket, TestStruct2
    from egui_states_test_bindings.enums import TestEnum2

    st.wire.set(
        WirePacket(
            -(2**63),
            2**64 - 1,
            "žluťoučký 🦀",
            -(2**31),
            [0, 32768, 65535],
            TestStruct2(True, 65535, "中"),
            TestEnum2.Z,
        )
    )
    st.wire.connect(lambda packet: st.wire_echo.set(packet, update=True))
    st.value.set(7)
    st.items.set([1, 2])
    st.cached.set(np.array([8, 9], dtype=np.uint8), cache=True)
    st.cached_multi[91].set(np.array([500, 600], dtype=np.uint16), cache=True)
    st.value.connect(lambda value: st.callback_value.set(value, update=True))

    def command(number):
        if number == 1:
            st.value.set(21, update=True)
        elif number == 2:
            st.items.add_item(st.value.get(), update=True)
            st.map.set_item(9, 900, update=True)
        elif number == 3:
            states.data_take.take_samples.set(np.array([-1.25, 0, 2.5], dtype=np.float32), update=True)
            states.data_multi_take.samples[513].set(np.array([9.5, -2.25], dtype=np.float32), update=True)
            states.data_multi_take.nested.buffer[24].set(np.array([0, 65535], dtype=np.uint16), update=True)
            st.take.set("payload", update=True)
            st.empty.set(update=True)
            st.data.set(np.array([0, 1, 255], dtype=np.uint8), update=True)
            st.multi[7].set(np.array([100, 200], dtype=np.uint16), update=True)
            st.multi[42].set(np.array([], dtype=np.uint16), update=True)
        elif number == 4:
            st.take.set("", update=True)
            st.data.set(np.array([], dtype=np.uint8), update=True)
        elif number in {10, 11, 12, 13}:

            def send(second):
                if number == 10:
                    st.take.set("" if second else "first", blocking=True, update=True)
                elif number == 11:
                    st.empty.set(blocking=True, update=True)
                elif number == 12:
                    st.data.set(np.array([] if second else [1, 2], dtype=np.uint8), blocking=True, update=True)
                else:
                    st.multi[73].set(
                        np.array([] if second else [100, 200], dtype=np.uint16), blocking=True, update=True
                    )

            send(False)
            st.phase.set(1, update=True)
            send(True)
            st.phase.set(2, update=True)
        elif number == 20:
            st.cached.set(np.array([31, 32], dtype=np.uint8), cache=True, update=True)
            st.cached_multi.remove_index(91, update=True)
            st.cached_multi[513].set(np.array([17], dtype=np.uint16), cache=True, update=True)
            st.phase.set(20, update=True)
        elif number == 21:
            st.cached.set(np.array([41], dtype=np.uint8), cache=False, update=True)
            st.cached_multi.reset(update=True)
            st.phase.set(21, update=True)
        elif number == 22:
            st.multi[7].set(np.array([7], dtype=np.uint16), update=True)
            st.multi[900].set(np.array([9], dtype=np.uint16), update=True)
            st.multi.remove_index(7, update=True)
            st.phase.set(22, update=True)
        elif number == 23:
            st.multi.reset(update=True)
            st.phase.set(23, update=True)
        elif number in (24, 25):
            st.multi[7].set(np.array([7], dtype=np.uint16), blocking=True, update=True)
            if number == 24:
                st.multi.remove_index(7, update=True)
            else:
                st.multi.reset(update=True)
            st.phase.set(24, update=True)
            st.multi[513].set(np.array([9], dtype=np.uint16), blocking=True, update=True)
            st.phase.set(25, update=True)
        elif number == 99:
            st.phase.set(99, update=True)
        else:
            raise AssertionError(f"unknown command: {number}")

    st.command.connect(command)


def run_scenario(probe, scenario):
    """Run the server and its potentially blocking callbacks in a disposable process."""
    errors = []
    server = StatesServer(error_handler=errors.append, version=StatesServer.VERSION_HASH)
    from egui_states.logging import LogLevel

    server.logging.add_logger(LogLevel.Error, lambda message: errors.append(RuntimeError(message)))
    configure(server.states)
    port = free_port()
    try:
        server.start(port, (127, 0, 0, 1))
        run([probe, port, scenario], timeout=30)

    finally:
        server.stop()
        assert not errors, f"Unexpected callback errors: {errors!r}"
