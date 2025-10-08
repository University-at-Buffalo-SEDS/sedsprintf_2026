#include "sedsprintf.h"
// example.c
#include <stdio.h>
#include <string.h>
#include <inttypes.h>

// --- Optional: stash last TX buffer so we can demonstrate seds_router_receive() later
static uint8_t g_last_tx[256];
static size_t g_last_tx_len = 0;

// ---- A transmit function that "sends" bytes to another router (loopback demo)
static SedsResult loopback_tx(const uint8_t * bytes, size_t len, void * user)
{
    SedsRouter * remote = user;

    // Save a copy (demo only)
    if (len <= sizeof(g_last_tx))
    {
        memcpy(g_last_tx, bytes, len);
        g_last_tx_len = len;
    }
    else
    {
        g_last_tx_len = 0; // too big for demo buffer
    }

    // In a real system you would put bytes on UART/SPI/radio, etc.
    // For the demo, immediately feed them into the remote router:
    return seds_router_receive(remote, bytes, len);
}

// ---- A simple C endpoint handler: print metadata and decode f32 payloads
static SedsResult rx_handler(const SedsPacketView * pkt, void * user)
{
    (void) user;
    char data[seds_pkt_to_string_len(pkt)];
    SedsResult status = seds_pkt_to_string(pkt, data, sizeof(data));
    if (status != SEDS_OK)
    {
        printf("rx_handler: seds_pkt_to_string failed: %d\n", status);
        return status;
    }
    printf("rx_handler: received packet: %s\n", data);
    return SEDS_OK;
}

int main(void)
{
    // Router B: local handler for RADIO only
    SedsLocalEndpointDesc b_local_endpoint_handlers[] = {
        {SEDS_EP_RADIO, &rx_handler, NULL},
    };
    SedsRouter * router_b = seds_router_new(/*tx*/NULL, /*tx_user*/NULL,
                                                  b_local_endpoint_handlers, sizeof(b_local_endpoint_handlers) / sizeof(b_local_endpoint_handlers[0]));
    if (!router_b)
    {
        fprintf(stderr, "failed to create router_b\n");
        return 1;
    }

    // Router A: local handler for SD_CARD, transmit loops into router B
    const SedsLocalEndpointDesc a_local_endpoint_handlers[] = {
        {SEDS_EP_SD, &rx_handler, NULL},
    };
    SedsRouter * router_a = seds_router_new(&loopback_tx, /*tx_user=*/router_b,
                                            a_local_endpoint_handlers, sizeof(a_local_endpoint_handlers) / sizeof(a_local_endpoint_handlers[0]));
    if (!router_a)
    {
        fprintf(stderr, "failed to create router_a\n");
        seds_router_free(router_b);
        return 1;
    }

    // Log a GPS packet (3*f32). Per the schema the router will:
    //  - serialize once
    //  - call transmit (bytes -> router_b)
    //  - locally dispatch to endpoints present (SD on router_a)
    float gps[3] = {1.0f, 2.5f, 3.25f};
    const int rc = seds_router_log(router_a, SEDS_DT_GPS, gps, 3, /*timestamp=*/0);
    if (rc != SEDS_OK)
    {
        fprintf(stderr, "seds_router_log failed: %d\n", rc);
    }

    // (Optional) Demonstrate calling seds_router_receive() yourself with the last TX copy.
    if (g_last_tx_len)
    {
        printf("\nManual re-inject of last TX buffer into router_b via seds_router_receive()...\n");
        (void) seds_router_receive(router_b, g_last_tx, g_last_tx_len);
    }

    seds_router_free(router_a);
    seds_router_free(router_b);
    return 0;
}
