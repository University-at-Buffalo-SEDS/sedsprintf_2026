#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <unistd.h>

#include "telemetry_sim.h"

static void pump_nodes(SimNode **nodes, size_t count, unsigned rounds)
{
    for (unsigned i = 0; i < rounds; ++i)
    {
        for (size_t n = 0; n < count; ++n)
        {
            if (nodes[n] && nodes[n]->r)
            {
                (void)seds_router_process_tx_queue_with_timeout(nodes[n]->r, 5);
                (void)seds_router_process_rx_queue_with_timeout(nodes[n]->r, 5);
            }
        }
        usleep(1000);
    }
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

    SimNode gm, c1, c2;
    assert(node_init(&gm, &can_bus, "GrandMaster", 0, 0, 1) == SEDS_OK);
    assert(node_init(&c1, &can_bus, "Consumer1", 0, 0, 0) == SEDS_OK);
    assert(node_init(&c2, &can_bus, "Consumer2", 0, 0, 0) == SEDS_OK);

    int32_t gm_radio_side = seds_router_add_side_serialized(gm.r, "RADIO", 5, radio_side_tx, &gm, true);
    assert(gm_radio_side >= 0);

    SimNode *nodes[] = {&gm, &c1, &c2};
    pump_nodes(nodes, 3, 400);

    uint64_t c1_network_ms = 0;
    uint64_t c2_network_ms = 0;
    assert(seds_router_get_network_time_ms(c1.r, &c1_network_ms) == SEDS_OK);
    assert(seds_router_get_network_time_ms(c2.r, &c2_network_ms) == SEDS_OK);
    assert(c1_network_ms > 0);
    assert(c2_network_ms > 0);

    printf("board-topology timesync ok: c1=%llu c2=%llu\n",
           (unsigned long long)c1_network_ms,
           (unsigned long long)c2_network_ms);

    node_free(&c2);
    node_free(&c1);
    node_free(&gm);
    bus_free(&can_bus);

    return 0;
}
