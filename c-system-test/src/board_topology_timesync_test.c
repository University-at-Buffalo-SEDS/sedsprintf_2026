#include <assert.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <unistd.h>

#include "telemetry_sim.h"

#define TIMESYNC_PERIOD_MS 2000ULL

typedef struct
{
    const char *name;
    SimNode node;
    int is_grandmaster;
    uint64_t last_timesync_ms;
    uint64_t req_seq;
    pthread_t thread;
} BoardCtx;

static volatile int g_stop = 0;

static uint64_t board_now_ms(BoardCtx *b)
{
    return node_now_since_bus_ms(&b->node);
}

static void board_timesync_tick(BoardCtx *b)
{
    const uint64_t now = board_now_ms(b);
    if ((now - b->last_timesync_ms) < TIMESYNC_PERIOD_MS)
    {
        return;
    }
    b->last_timesync_ms = now;

    if (b->is_grandmaster)
    {
        const uint64_t ann[2] = {1ULL, now};
        assert(node_log(&b->node, SEDS_DT_TIME_SYNC_ANNOUNCE, ann, 2, sizeof(uint64_t)) == SEDS_OK);
    }
    else
    {
        const uint64_t req[2] = {++b->req_seq, now};
        assert(node_log(&b->node, SEDS_DT_TIME_SYNC_REQUEST, req, 2, sizeof(uint64_t)) == SEDS_OK);
    }
}

static void *board_thread_entry(void *arg)
{
    BoardCtx *b = (BoardCtx *)arg;
    while (!g_stop)
    {
        (void)seds_router_process_tx_queue_with_timeout(b->node.r, 5);
        (void)seds_router_process_rx_queue_with_timeout(b->node.r, 5);
        board_timesync_tick(b);
        usleep(1000);
    }

    for (int i = 0; i < 40; ++i)
    {
        (void)seds_router_process_tx_queue_with_timeout(b->node.r, 0);
        (void)seds_router_process_rx_queue_with_timeout(b->node.r, 0);
        usleep(1000);
    }
    return NULL;
}

static SedsResult radio_side_tx(const uint8_t *bytes, size_t len, void *user)
{
    (void)bytes;
    (void)len;
    (void)user;
    return SEDS_OK;
}

int main(void)
{
    SimBus can_bus;
    bus_init(&can_bus);

    BoardCtx gm = {.name = "GrandMaster", .is_grandmaster = 1, .last_timesync_ms = 0, .req_seq = 0};
    BoardCtx c1 = {.name = "Consumer1", .is_grandmaster = 0, .last_timesync_ms = 0, .req_seq = 0};
    BoardCtx c2 = {.name = "Consumer2", .is_grandmaster = 0, .last_timesync_ms = 0, .req_seq = 0};

    assert(node_init(&gm.node, &can_bus, gm.name, 0, 0, 1) == SEDS_OK);
    assert(node_init(&c1.node, &can_bus, c1.name, 0, 0, 0) == SEDS_OK);
    assert(node_init(&c2.node, &can_bus, c2.name, 0, 0, 0) == SEDS_OK);

    // Grandmaster has two sides: CAN ("BUS" from node_init) + RADIO.
    int32_t gm_radio_side = seds_router_add_side_serialized(gm.node.r, "RADIO", 5, radio_side_tx, &gm, true);
    assert(gm_radio_side >= 0);

    pthread_create(&gm.thread, NULL, board_thread_entry, &gm);
    pthread_create(&c1.thread, NULL, board_thread_entry, &c1);
    pthread_create(&c2.thread, NULL, board_thread_entry, &c2);

    // Let the per-board loops run for >2 periods.
    usleep(5500 * 1000);

    g_stop = 1;
    pthread_join(gm.thread, NULL);
    pthread_join(c1.thread, NULL);
    pthread_join(c2.thread, NULL);

    // GM should receive requests from both consumers.
    assert(gm.node.ts_request_hits >= 2);
    // Consumers should see GM announces and responses.
    assert(c1.node.ts_announce_hits >= 1);
    assert(c2.node.ts_announce_hits >= 1);
    assert(c1.node.ts_response_hits >= 1);
    assert(c2.node.ts_response_hits >= 1);

    printf("board-topology timesync ok: gm(req=%u ann=%u resp=%u) c1(ann=%u resp=%u) c2(ann=%u resp=%u)\n",
           gm.node.ts_request_hits, gm.node.ts_announce_hits, gm.node.ts_response_hits,
           c1.node.ts_announce_hits, c1.node.ts_response_hits,
           c2.node.ts_announce_hits, c2.node.ts_response_hits);

    node_free(&c2.node);
    node_free(&c1.node);
    node_free(&gm.node);
    bus_free(&can_bus);

    return 0;
}
