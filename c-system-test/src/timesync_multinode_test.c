#include <assert.h>
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
                (void)seds_router_process_tx_queue_with_timeout(nodes[n]->r, 0);
                (void)seds_router_process_rx_queue_with_timeout(nodes[n]->r, 0);
            }
        }
        usleep(1000);
    }
}

static void send_announce(SimNode *src, uint64_t priority)
{
    const uint64_t ann[2] = {priority, node_now_since_bus_ms(src)};
    assert(node_log(src, SEDS_DT_TIME_SYNC_ANNOUNCE, ann, 2, sizeof(uint64_t)) == SEDS_OK);
}

static void send_request(SimNode *src, uint64_t seq)
{
    const uint64_t req[2] = {seq, node_now_since_bus_ms(src)};
    assert(node_log(src, SEDS_DT_TIME_SYNC_REQUEST, req, 2, sizeof(uint64_t)) == SEDS_OK);
}

static void scenario_grandmaster_and_consumers(void)
{
    SimBus bus;
    bus_init(&bus);

    SimNode gm, c1, c2;
    assert(node_init(&gm, &bus, "GM", 0, 0, 1) == SEDS_OK);
    assert(node_init(&c1, &bus, "C1", 0, 0, 0) == SEDS_OK);
    assert(node_init(&c2, &bus, "C2", 0, 0, 0) == SEDS_OK);

    SimNode *nodes[] = {&gm, &c1, &c2};

    send_announce(&gm, 1);
    send_announce(&gm, 1);
    pump_nodes(nodes, 3, 40);

    send_request(&c1, 1001);
    send_request(&c2, 1002);
    pump_nodes(nodes, 3, 60);

    assert(gm.ts_request_hits >= 2);
    assert(c1.ts_announce_hits >= 1);
    assert(c2.ts_announce_hits >= 1);
    assert(c1.ts_response_hits >= 1);
    assert(c2.ts_response_hits >= 1);

    node_free(&c2);
    node_free(&c1);
    node_free(&gm);
    bus_free(&bus);
}

static void scenario_failover_primary_to_backup(void)
{
    SimBus bus;
    bus_init(&bus);

    SimNode primary, backup, consumer;
    assert(node_init(&primary, &bus, "GM_PRIMARY", 0, 0, 1) == SEDS_OK);
    assert(node_init(&backup, &bus, "GM_BACKUP", 0, 0, 0) == SEDS_OK);
    assert(node_init(&consumer, &bus, "CONSUMER", 0, 0, 0) == SEDS_OK);

    SimNode *nodes[] = {&primary, &backup, &consumer};

    send_announce(&primary, 1);
    pump_nodes(nodes, 3, 30);

    send_request(&consumer, 2001);
    pump_nodes(nodes, 3, 60);

    const unsigned consumer_resp_before = consumer.ts_response_hits;
    assert(consumer_resp_before >= 1);

    node_free(&primary);
    backup.is_time_source = 1;

    const unsigned backup_req_before = backup.ts_request_hits;

    send_announce(&backup, 2);
    send_request(&consumer, 2002);
    pump_nodes(nodes, 3, 80);

    assert(backup.ts_request_hits > backup_req_before);
    assert(consumer.ts_response_hits > consumer_resp_before);

    node_free(&consumer);
    node_free(&backup);
    bus_free(&bus);
}

int main(void)
{
    scenario_grandmaster_and_consumers();
    scenario_failover_primary_to_backup();
    printf("timesync multinode scenarios passed\n");
    return 0;
}
