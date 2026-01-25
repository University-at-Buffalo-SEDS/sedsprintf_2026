# C/C++ Usage

The C API is exposed via `C-Headers/sedsprintf.h` and a static library built by Cargo.

## CMake integration (recommended)

```
# Example: building for an embedded target
set(SEDSPRINTF_RS_TARGET "thumbv7em-none-eabihf" CACHE STRING "" FORCE)
set(SEDSPRINTF_EMBEDDED_BUILD ON CACHE BOOL "" FORCE)

# set the sender name
set(SEDSPRINTF_RS_DEVICE_IDENTIFIER "FC26_MAIN" CACHE STRING "" FORCE)

# optional compile-time env overrides
set(SEDSPRINTF_RS_MAX_STACK_PAYLOAD "256" CACHE STRING "" FORCE)
set(SEDSPRINTF_RS_ENV_MAX_QUEUE_SIZE "65536" CACHE STRING "" FORCE)

add_subdirectory(${CMAKE_SOURCE_DIR}/sedsprintf_rs sedsprintf_rs_build)

target_link_libraries(${CMAKE_PROJECT_NAME} PRIVATE sedsprintf_rs::sedsprintf_rs)
```

Important CMake variables:
- `SEDSPRINTF_EMBEDDED_BUILD` (ON/OFF)
- `SEDSPRINTF_RS_TARGET` (Rust target triple)
- `SEDSPRINTF_RS_DEVICE_IDENTIFIER`
- `SEDSPRINTF_RS_MAX_STACK_PAYLOAD`
- `SEDSPRINTF_RS_ENV_<KEY>` for any config env var

## Minimal C example

```
#include "sedsprintf.h"

static uint64_t now_ms(void *user) { (void)user; return 0; }
static SedsResult tx_send(const uint8_t *bytes, size_t len, void *user)
{
    (void)bytes; (void)len; (void)user;
    return SEDS_OK;
}

static SedsResult on_packet(const SedsPacketView *pkt, void *user)
{
    (void)user;
    char buf[seds_pkt_to_string_len(pkt)];
    seds_pkt_to_string(pkt, buf, sizeof(buf));
    return SEDS_OK;
}

int main(void)
{
    const SedsLocalEndpointDesc locals[] = {
        { .endpoint = SEDS_EP_SD_CARD, .packet_handler = on_packet, .user = NULL },
    };

    SedsRouter *r = seds_router_new(
        Seds_RM_Sink,
        tx_send,
        NULL,
        now_ms,
        locals,
        sizeof(locals) / sizeof(locals[0])
    );

    float data[3] = {1.0f, 2.0f, 3.0f};
    seds_router_log(r, SEDS_DT_GPS_DATA, data, sizeof(data));
    seds_router_process_all_queues(r);

    seds_router_free(r);
    return 0;
}
```

See `c-example-code/` for a more complete example.

## Embedded allocator hooks
Bare-metal builds must provide:
- `void *telemetryMalloc(size_t)`
- `void telemetryFree(void *)`
- `void seds_error_msg(const char *, size_t)`

A simple stub is shown in `README.md` and can be adapted for your platform.
