#include "sedsprintf.h"
// example.c
#include <stdio.h>
#include <string.h>
#include <inttypes.h>

// Keep these in sync with your Rust enums:
// DataType:     0=GpsData, 1=ImuData, 2=BatteryStatus, 3=SystemStatus
// DataEndpoint: 0=SdCard,  1=Radio

// --- Optional: stash last TX buffer so we can demonstrate seds_router_receive() later
static uint8_t g_last_tx[256];
static size_t  g_last_tx_len = 0;

// ---- A transmit function that "sends" bytes to another router (loopback demo)
static int loopback_tx(const uint8_t* bytes, size_t len, void* user) {
  SedsRouter* remote = user;

  // Save a copy (demo only)
  if (len <= sizeof(g_last_tx)) {
    memcpy(g_last_tx, bytes, len);
    g_last_tx_len = len;
  } else {
    g_last_tx_len = 0; // too big for demo buffer
  }

  // In a real system you would put bytes on UART/SPI/radio, etc.
  // For the demo, immediately feed them into the remote router:
  return seds_router_receive(remote, bytes, len);
}

// ---- A simple C endpoint handler: print metadata and decode f32 payloads
static int print_handler(const SedsPacketView* pkt, void* user) {
  (void)user;

  printf("[C handler] ty=%" PRIu32 ", size=%zu, ts=%" PRIu64 ", endpoints=[",
         pkt->ty, pkt->data_size, pkt->timestamp);
  for (size_t i = 0; i < pkt->num_endpoints; ++i) {
    printf("%s%" PRIu32, (i ? "," : ""), pkt->endpoints[i]);
  }
  printf("]\n");

  // For GPS (3*f32 = 12 bytes), demonstrate decoding to floats:
  if (pkt->ty == SEDS_DT_GPS && pkt->payload_len == 12) {
    float vals[3];
    int rc = seds_pkt_get_f32(pkt, vals, 3);
    if (rc == SEDS_OK) {
      printf("  payload(f32): [%g, %g, %g]\n", vals[0], vals[1], vals[2]);
    } else {
      printf("  seds_pkt_get_f32 failed: %d\n", rc);
    }
  } else {
    // Otherwise just print first few bytes
    size_t n = pkt->payload_len < 8 ? pkt->payload_len : 8;
    printf("  payload(bytes, first %zu):", n);
    for (size_t i = 0; i < n; ++i) {
      printf(" %02x", (unsigned)pkt->payload[i]);
    }
    printf("%s\n", pkt->payload_len > n ? " ..." : "");
  }
  return SEDS_OK;
}

int main(void) {
  // Router B: local handler for RADIO only
  SedsHandlerDesc b_handlers[] = {
    { SEDS_EP_RADIO,  &print_handler,  NULL },
  };
  SedsRouter* router_b = seds_router_new(/*tx*/NULL, /*tx_user*/NULL,
                                         b_handlers, sizeof(b_handlers)/sizeof(b_handlers[0]));
  if (!router_b) {
    fprintf(stderr, "failed to create router_b\n");
    return 1;
  }

  // Router A: local handler for SD_CARD, transmit loops into router B
  SedsHandlerDesc a_handlers[] = {
    { SEDS_EP_SD,   &print_handler,  NULL },
  };
  SedsRouter* router_a = seds_router_new(&loopback_tx, /*tx_user=*/router_b,
                                         a_handlers, sizeof(a_handlers)/sizeof(a_handlers[0]));
  if (!router_a) {
    fprintf(stderr, "failed to create router_a\n");
    seds_router_free(router_b);
    return 1;
  }

  // Log a GPS packet (3*f32). Per your schema the router will:
  //  - serialize once
  //  - call transmit (bytes -> router_b)
  //  - locally dispatch to endpoints present (SD on router_a)
  float gps[3] = { 1.0f, 2.5f, 3.25f };
  int rc = seds_router_log_f32(router_a, SEDS_DT_GPS, gps, 3, /*timestamp=*/0);
  if (rc != SEDS_OK) {
    fprintf(stderr, "seds_router_log_f32 failed: %d\n", rc);
  }

  // (Optional) Demonstrate calling seds_router_receive() yourself with the last TX copy.
  if (g_last_tx_len) {
    printf("\nManual re-inject of last TX buffer into router_b via seds_router_receive()...\n");
    (void)seds_router_receive(router_b, g_last_tx, g_last_tx_len);
  }

  seds_router_free(router_a);
  seds_router_free(router_b);
  return 0;
}
