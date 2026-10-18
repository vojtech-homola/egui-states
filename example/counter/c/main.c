/* Counter server written in C against the egui-states C ABI.
 *
 * Usage: counter-c [PORT] [--smoke-test]
 *
 * It logs every count the GUI signals and restores zero on Reset, until it is
 * stopped with Ctrl+C. With --smoke-test it instead exits when the client
 * disconnects, successfully after at least two resets, and fails after
 * SMOKE_TEST_SECONDS. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "egui_states.h"
#include "counter_state.h"

#define SMOKE_TEST_SECONDS 20

#define CHECK(call)                                                            \
    do {                                                                       \
        egui_states_status_t check_status = (call);                            \
        if (check_status != EGUI_STATES_OK) {                                  \
            fprintf(stderr, "%s failed (%d): %s\n", #call, (int)check_status,  \
                    egui_states_last_error());                                 \
            exit(1);                                                           \
        }                                                                      \
    } while (0)

/* Replaces the count, optionally asking the GUI to repaint. */
static void set_count(egui_states_server_t *server, egui_states_writer_t *writer,
                      int32_t count, uint32_t flags) {
    CHECK(egui_states_writer_clear(writer));
    CHECK(egui_states_write_i32(writer, count));
    CHECK(egui_states_value_set(server, COUNTER_STATE_COUNT_ID, writer, flags));
}

int main(int argc, char **argv) {
    long port = argc > 1 ? strtol(argv[1], NULL, 10) : 8092;
    int smoke_test = argc > 2 && strcmp(argv[2], "--smoke-test") == 0;
    egui_states_server_t *server = NULL;
    egui_states_writer_t *writer = NULL;
    egui_states_reader_t *reader = NULL;
    time_t deadline = time(NULL) + SMOKE_TEST_SECONDS;
    int resets = 0;
    int result = 1;

    if (port <= 0 || port > 65535) {
        fprintf(stderr, "invalid port %ld\n", port);
        return 2;
    }

    /* Register the generated state tree, then end registration. */
    CHECK(egui_states_server_new(NULL, &server));
    CHECK(counter_state_register(server));
    CHECK(egui_states_server_finalize(server));
    CHECK(egui_states_writer_new(&writer));
    CHECK(egui_states_reader_new(&reader));

    /* A non-default value shows that the client received the server's state. */
    set_count(server, writer, -1, 0);

    CHECK(egui_states_server_subscribe(server, COUNTER_STATE_COUNT_ID, 0));
    CHECK(egui_states_server_subscribe(server, COUNTER_STATE_RESET_ID, 0));
    CHECK(egui_states_server_subscribe(server, EGUI_STATES_ID_ON_DISCONNECT, 0));
    CHECK(egui_states_server_start(server, (uint16_t)port, "127.0.0.1", NULL));
    printf("listening %ld\n", port);
    fflush(stdout);

    while (!smoke_test || time(NULL) < deadline) {
        egui_states_event_t *event = NULL;
        uint64_t id = 0;
        egui_states_status_t status = egui_states_server_wait_event(server, 500, &event);
        if (status == EGUI_STATES_ERROR_TIMEOUT) {
            continue;
        }
        CHECK(status);
        CHECK(egui_states_event_id(event, &id));

        if (id == COUNTER_STATE_COUNT_ID) {
            int32_t count = 0;
            CHECK(egui_states_event_value(event, reader));
            CHECK(egui_states_read_i32(reader, &count));
            printf("count: %d\n", (int)count);
            fflush(stdout);
        } else if (id == COUNTER_STATE_RESET_ID) {
            set_count(server, writer, 0, EGUI_STATES_UPDATE);
            resets += 1;
        } else if (id == EGUI_STATES_ID_ON_DISCONNECT && smoke_test) {
            egui_states_event_free(event);
            result = resets >= 2 ? 0 : 1;
            break;
        }
        /* Freeing the event lets the next change of the same state arrive. */
        egui_states_event_free(event);
    }

    if (result != 0) {
        fprintf(stderr, "stopped after %d resets\n", resets);
    }
    CHECK(egui_states_server_close_events(server));
    CHECK(egui_states_server_stop(server));
    egui_states_reader_free(reader);
    egui_states_writer_free(writer);
    egui_states_server_free(server);
    return result;
}
