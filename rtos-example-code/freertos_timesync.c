// FreeRTOS time sync example (C)
// NOTE: adjust includes for your platform.
#include "sedsprintf.h"
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "FreeRTOS.h"
#include "task.h"

static uint64_t ticks_to_ms(TickType_t ticks)
{
    return (uint64_t) ticks * (1000ULL / configTICK_RATE_HZ);
}

static uint64_t now_ms(void * user)
{
    (void) user;
    return ticks_to_ms(xTaskGetTickCount());
}

static SedsResult tx_send(const uint8_t * bytes, size_t len, void * user)
{
    (void) bytes;
    (void) len;
    (void) user;
    return SEDS_OK;
}

static SedsResult on_timesync(const SedsPacketView * pkt, void * user)
{
    (void) user;
    if (!pkt || !pkt->payload) return SEDS_ERR;

    if (pkt->ty == SEDS_DT_TIME_SYNC_REQUEST && pkt->payload_len >= 16)
    {
        uint64_t seq, t1;
        memcpy(&seq, pkt->payload, sizeof(seq));
        memcpy(&t1, pkt->payload + 8, sizeof(t1));
        uint64_t t2 = now_ms(NULL);
        uint64_t t3 = now_ms(NULL);
        const uint64_t resp[4] = {seq, t1, t2, t3};
        seds_router_log_ts((SedsRouter *) pkt->user, SEDS_DT_TIME_SYNC_RESPONSE, t3, resp, 4);
    }
    return SEDS_OK;
}

static void compute_offset_delay(uint64_t t1, uint64_t t2, uint64_t t3, uint64_t t4,
                                 int64_t * offset_ms, uint64_t * delay_ms)
{
    const int64_t o = ((int64_t) (t2 - t1) + (int64_t) (t3 - t4)) / 2;
    const int64_t d = (int64_t) (t4 - t1) - (int64_t) (t3 - t2);
    *offset_ms = o;
    *delay_ms = d < 0 ? 0 : (uint64_t) d;
}

void timesync_task(void * arg)
{
    (void) arg;
    const SedsLocalEndpointDesc locals[] = {
        {.endpoint = SEDS_EP_TIME_SYNC, .packet_handler = on_timesync, .user = NULL},
    };
    SedsRouter * r = seds_router_new(
        Seds_RM_Sink,
        now_ms,
        NULL,
        locals,
        sizeof(locals) / sizeof(locals[0])
    );
    seds_router_add_side_serialized(r, "RADIO", 5, tx_send, NULL, true);

    uint64_t seq = 1;
    for (;;)
    {
        const uint64_t t1 = now_ms(NULL);
        const uint64_t req[2] = {seq++, t1};
        seds_router_log_ts(r, SEDS_DT_TIME_SYNC_REQUEST, t1, req, 2);

        seds_router_process_all_queues_with_timeout(r, 5);

        // Example: use last response timestamps to compute offset/delay.
        // Real code should capture t2/t3/t4 and apply a filtered offset.
        {
            const uint64_t t2 = t1 + 5;
            const uint64_t t3 = t1 + 7;
            const uint64_t t4 = t1 + 12;
            int64_t offset_ms = 0;
            uint64_t delay_ms = 0;
            compute_offset_delay(t1, t2, t3, t4, &offset_ms, &delay_ms);
            (void) offset_ms;
            (void) delay_ms;
        }

        vTaskDelay(pdMS_TO_TICKS(1000));
    }
}
