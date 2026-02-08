#include "sedsprintf.h"
#include <stdio.h>
#include <string.h>
#include <sys/time.h>

static uint64_t host_now_ms(void * user)
{
    (void) user;
    struct timeval tv;
    gettimeofday(&tv, NULL);
    return (uint64_t) tv.tv_sec * 1000ULL + (uint64_t) (tv.tv_usec / 1000ULL);
}

static SedsResult tx_send(const uint8_t * bytes, size_t len, void * user)
{
    (void) bytes;
    (void) len;
    (void) user;
    return SEDS_OK;
}

static SedsResult on_packet(const SedsPacketView * pkt, void * user)
{
    (void) user;
    char buf[seds_pkt_to_string_len(pkt)];
    if (seds_pkt_to_string(pkt, buf, sizeof(buf)) == SEDS_OK)
    {
        printf("%s\n", buf);
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

int main(void)
{
    const SedsLocalEndpointDesc locals[] = {
        {.endpoint = SEDS_EP_RADIO, .packet_handler = on_packet, .user = NULL},
        {.endpoint = SEDS_EP_SD_CARD, .packet_handler = on_packet, .user = NULL},
        {.endpoint = SEDS_EP_TIME_SYNC, .packet_handler = on_packet, .user = NULL},
    };

    SedsRouter * r = seds_router_new(
        Seds_RM_Sink,
        host_now_ms,
        NULL,
        locals,
        sizeof(locals) / sizeof(locals[0])
    );
    if (!r)
    {
        fprintf(stderr, "router init failed\n");
        return 1;
    }
    seds_router_add_side_serialized(r, "TX", 2, tx_send, NULL, true);

    // ---- Log all standard schema types ----
    const float gps[3] = {37.7749f, -122.4194f, 30.0f};
    seds_router_log(r, SEDS_DT_GPS_DATA, gps, 3);

    const float imu[6] = {0.1f, 0.2f, 0.3f, 1.1f, 1.2f, 1.3f};
    seds_router_log(r, SEDS_DT_IMU_DATA, imu, 6);

    const float batt[2] = {12.5f, 1.8f};
    seds_router_log(r, SEDS_DT_BATTERY_STATUS, batt, 2);

    const bool status[1] = {true};
    seds_router_log(r, SEDS_DT_SYSTEM_STATUS, status, 1);

    const float baro[3] = {1013.2f, 24.5f, 0.0f};
    seds_router_log(r, SEDS_DT_BAROMETER_DATA, baro, 3);

    seds_router_log_cstr(r, SEDS_DT_MESSAGE_DATA, "hello from C timesync example");

    const uint8_t none[0];
    seds_router_log(r, SEDS_DT_HEARTBEAT, none, 0);

    // ---- Time sync packets ----
    const uint64_t t1 = host_now_ms(NULL);
    const uint64_t announce[2] = {10ULL, t1};
    seds_router_log_ts(r, SEDS_DT_TIME_SYNC_ANNOUNCE, t1, announce, 2);

    const uint64_t req[2] = {1ULL, t1};
    seds_router_log_ts(r, SEDS_DT_TIME_SYNC_REQUEST, t1, req, 2);

    const uint64_t t2 = t1 + 5;
    const uint64_t t3 = t1 + 7;
    const uint64_t resp[4] = {1ULL, t1, t2, t3};
    seds_router_log_ts(r, SEDS_DT_TIME_SYNC_RESPONSE, t3, resp, 4);

    const uint64_t t4 = t1 + 12;
    int64_t offset_ms = 0;
    uint64_t delay_ms = 0;
    compute_offset_delay(t1, t2, t3, t4, &offset_ms, &delay_ms);
    printf("timesync sample: offset_ms=%lld delay_ms=%llu\n",
           (long long) offset_ms, (unsigned long long) delay_ms);

    seds_router_process_all_queues_with_timeout(r, 0);
    seds_router_free(r);
    return 0;
}
