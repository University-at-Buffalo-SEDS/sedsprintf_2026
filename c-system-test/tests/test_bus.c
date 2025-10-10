#include <assert.h>
#include <stdio.h>
#include "../include/telemetry_sim.h"
#include "sedsprintf.h"

static void series(float *dst, size_t n, float base) {
    for (size_t i = 0; i < n; ++i) dst[i] = base + (float)i * 0.5f;
}

int main(void) {
    SimBus bus; bus_init(&bus);

    // A: RADIO, B: SD, C: none
    SimNode A, B, C;
    assert(node_init(&A, &bus, "A", 1, 0) == SEDS_OK);
    assert(node_init(&B, &bus, "B", 0, 1) == SEDS_OK);
    assert(node_init(&C, &bus, "C", 0, 0) == SEDS_OK);

    float f[6];

    // Send 3 GPS samples from A
    series(f, 3, 10.0f);
    assert(node_log(&A, SEDS_DT_GPS, f, 3) == SEDS_OK);

    // Send 6 IMU samples from B
    series(f, 6, 0.0f);
    assert(node_log(&B, SEDS_DT_IMU, f, 6) == SEDS_OK);

    // Send 2 BATTERY samples from C
    series(f, 2, 3.7f);
    assert(node_log(&C, SEDS_DT_BATTERY, f, 2) == SEDS_OK);

    // Expectations:
    // - No duplicates (each emitted packet delivered once to each *interested* receiver)
    // - RADIO only fires on A; SD only fires on B.
    // NOTE: Whether a given data type hits RADIO/SD depends on your router config.
    // For this test we assume GPS/IMU/BATTERY are routed to both RADIO and SD.
    // So A.radio_hits should be 3 + 6 + 2 = 11
    // And B.sd_hits   should be 3 + 6 + 2 = 11
    // C has no local endpoints -> 0

    printf("A.radio_hits=%u, B.sd_hits=%u, C.radio_hits=%u, C.sd_hits=%u\n",
           A.radio_hits, B.sd_hits, C.radio_hits, C.sd_hits);

    assert(A.radio_hits == 11);
    assert(B.sd_hits    == 11);
    assert(C.radio_hits == 0);
    assert(C.sd_hits    == 0);

    node_free(&A); node_free(&B); node_free(&C);
    bus_free(&bus);
    return 0;
}
