#ifndef SEDSPRINTF_C_H
#define SEDSPRINTF_C_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {

#endif


// Keep in sync with Rust's DataType order
typedef enum SedsDataType
{
    SEDS_DT_GPS,
    SEDS_DT_IMU,
    SEDS_DT_BATTERY,
    SEDS_DT_SYSTEM,
} SedsDataType;

// Keep in sync with Rust's DataEndpoint order
typedef enum SedsDataEndpoint
{
    SEDS_EP_SD,
    SEDS_EP_RADIO,
} SedsDataEndpoint;

// ---- Status codes (0 = OK) ----
enum
{
    SEDS_OK = 0,
    SEDS_ERR = -1,
    SEDS_BAD_ARG = -2,
    SEDS_INVALID_TYPE = -3,
    SEDS_SIZE_MISMATCH = -4,
    SEDS_DESERIALIZE = -5
};

// ---- Opaque handle to the Router ----
typedef struct SedsRouter SedsRouter;

// ---- Enums as u32 (must match Rust order) ----
// DataType: 0=GpsData, 1=ImuData, 2=BatteryStatus, 3=SystemStatus
// DataEndpoint: 0=SdCard, 1=Radio

// ---- Lightweight view of a telemetry packet (valid only during the callback) ----
typedef struct SedsPacketView
{
    uint32_t ty; // DataType as u32
    size_t data_size; // bytes
    const uint32_t * endpoints; // array of DataEndpoint (as u32)
    size_t num_endpoints;
    uint64_t timestamp; // seconds since epoch
    const uint8_t * payload; // bytes
    size_t payload_len; // == data_size
} SedsPacketView;

// ---- Callback types ----
typedef int (* SedsTransmitFn)(const uint8_t * bytes, size_t len, void * user);

typedef int (* SedsEndpointHandlerFn)(const SedsPacketView * pkt, void * user);

// Handler descriptor for construction
typedef struct SedsHandlerDesc
{
    uint32_t endpoint; // DataEndpoint as u32
    SedsEndpointHandlerFn handler;
    void * user;
} SedsHandlerDesc;

// ---- API ----

// Create a router with optional transmit callback and a static array of handlers.
// Returns NULL on failure.
SedsRouter * seds_router_new(SedsTransmitFn tx,
                             void * tx_user,
                             const SedsHandlerDesc * handlers,
                             size_t n_handlers);

// Destroy a router (safe to pass NULL).
void seds_router_free(SedsRouter * r);

// Log (bytes) via router.log_bytes()
int seds_router_log_bytes(SedsRouter * r,
                          uint32_t ty, // DataType
                          const uint8_t * data,
                          size_t len, // must match schema size
                          uint64_t timestamp);

// Log (f32 slice) via router.log_f32()
int seds_router_log_f32(SedsRouter * r,
                        uint32_t ty, // DataType
                        const float * vals,
                        size_t n_vals, // n * 4 must match schema size
                        uint64_t timestamp);

// Receive a serialized wire buffer and locally dispatch.
int seds_router_receive(SedsRouter * r,
                        const uint8_t * bytes,
                        size_t len);

// ---- Optional helpers for C handlers ----

// Decode payload as f32 (len == n * 4); returns 0 on success.
int seds_pkt_get_f32(const SedsPacketView * pkt, float * out, size_t n);

#ifdef __cplusplus
}
#endif

#endif // SEDSPRINTF_C_H
