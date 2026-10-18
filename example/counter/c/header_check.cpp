// Compiled as C++ by the `counter_c` smoke test to check that both headers are
// valid C++ as well as C.
#include "egui_states.h"
#include "counter_state.h"

egui_states_status_t register_counter(egui_states_server_t *server) {
    return counter_state_register(server);
}
