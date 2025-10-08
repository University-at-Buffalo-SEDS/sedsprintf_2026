#ifndef SEDSPRINTF_C_H
#define SEDSPRINTF_C_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

// Keep in sync with Rust's DataType order
typedef enum SedsDataType {
    SEDS_DT_GPS,
    SEDS_DT_IMU,
    SEDS_DT_BATTERY,
    SEDS_DT_SYSTEM,
} SedsDataType;

// Keep in sync with Rust's DataEndpoint order
typedef enum SedsDataEndpoint {
    SEDS_EP_SD,
    SEDS_EP_RADIO,
} SedsDataEndpoint;

// ---- Status codes (0 = OK) ----
enum {
    SEDS_OK = 0,
    SEDS_ERR = -1,
    SEDS_BAD_ARG = -2,
    SEDS_INVALID_TYPE = -3,
    SEDS_SIZE_MISMATCH = -4,
    SEDS_DESERIALIZE = -5
};

// ---- Opaque handle to the Router ----
typedef struct SedsRouter SedsRouter;

// ---- Lightweight view of a telemetry packet (valid only during the callback) ----
typedef struct SedsPacketView {
    uint32_t ty;              // DataType as u32
    size_t   data_size;       // bytes
    const uint32_t *endpoints;// array of DataEndpoint (as u32)
    size_t   num_endpoints;
    uint64_t timestamp;       // seconds since epoch (or your chosen unit)
    const uint8_t *payload;   // bytes
    size_t   payload_len;     // == data_size
} SedsPacketView;

// ---- Callback types ----
typedef int (*SedsTransmitFn)(const uint8_t *bytes, size_t len, void *user);
typedef int (*SedsEndpointHandlerFn)(const SedsPacketView *pkt, void *user);

// Handler descriptor for construction
typedef struct SedsHandlerDesc {
    uint32_t endpoint;             // DataEndpoint as u32
    SedsEndpointHandlerFn handler;
    void *user;
} SedsHandlerDesc;

// ---- API ----

// Create a router with optional transmit callback and a static array of handlers.
// Returns NULL on failure.
SedsRouter *seds_router_new(SedsTransmitFn tx,
                            void *tx_user,
                            const SedsHandlerDesc *handlers,
                            size_t n_handlers);

// Destroy a router (safe to pass NULL).
void seds_router_free(SedsRouter *r);

// ---------- Generic typed logging (NEW) ----------

typedef enum SedsElemKind {
    SEDS_EK_UNSIGNED = 0,  // u8/u16/u32/u64/...
    SEDS_EK_SIGNED   = 1,  // i8/i16/i32/i64/...
    SEDS_EK_FLOAT    = 2   // f32/f64
} SedsElemKind;

/**
 * Log a generic typed slice. The router will encode the slice to little-endian bytes
 * based on (elem_kind, elem_size). The total byte size (count * elem_size) must
 * exactly match the schema size for `ty`.
 *
 * elem_size must be one of {1,2,4,8}. Returns 0 on success.
 */
int seds_router_log_typed(SedsRouter *r,
                          uint32_t ty,               // DataType
                          const void *data,          // pointer to first element
                          size_t count,              // number of elements
                          size_t elem_size,          // 1,2,4,8
                          SedsElemKind elem_kind,    // unsigned/signed/float
                          uint64_t timestamp);

// ---------- Convenience: legacy explicit forms (still useful) ----------

// Log raw bytes (already little-endian as needed). len must match schema size.
int seds_router_log_bytes(SedsRouter *r,
                          uint32_t ty,
                          const uint8_t *data,
                          size_t len,
                          uint64_t timestamp);

// Log f32 slice; n_vals*4 must match schema size.
int seds_router_log_f32(SedsRouter *r,
                        uint32_t ty,
                        const float *vals,
                        size_t n_vals,
                        uint64_t timestamp);

// Receive a serialized wire buffer and locally dispatch.
int seds_router_receive(SedsRouter *r,
                        const uint8_t *bytes,
                        size_t len);

// ---- Optional helpers for C handlers ----

// Decode payload as f32 (len == n * 4); returns 0 on success.
int seds_pkt_get_f32(const SedsPacketView *pkt, float *out, size_t n);

#ifdef __cplusplus
} // extern "C"
#endif

// =================== Header-only helpers for nicer call sites ===================

// ---------- C++ templates ----------
#ifdef __cplusplus
extern "C" {
#endif

// internal selector: map C++ types to (kind, size)
#if defined(__cplusplus)
namespace seds_detail {
template <typename T> struct elem_traits;
template <> struct elem_traits<uint8_t> { static constexpr SedsElemKind kind = SEDS_EK_UNSIGNED; static constexpr size_t size = 1; };
template <> struct elem_traits<uint16_t>{ static constexpr SedsElemKind kind = SEDS_EK_UNSIGNED; static constexpr size_t size = 2; };
template <> struct elem_traits<uint32_t>{ static constexpr SedsElemKind kind = SEDS_EK_UNSIGNED; static constexpr size_t size = 4; };
template <> struct elem_traits<uint64_t>{ static constexpr SedsElemKind kind = SEDS_EK_UNSIGNED; static constexpr size_t size = 8; };
template <> struct elem_traits<int8_t>  { static constexpr SedsElemKind kind = SEDS_EK_SIGNED;   static constexpr size_t size = 1; };
template <> struct elem_traits<int16_t> { static constexpr SedsElemKind kind = SEDS_EK_SIGNED;   static constexpr size_t size = 2; };
template <> struct elem_traits<int32_t> { static constexpr SedsElemKind kind = SEDS_EK_SIGNED;   static constexpr size_t size = 4; };
template <> struct elem_traits<int64_t> { static constexpr SedsElemKind kind = SEDS_EK_SIGNED;   static constexpr size_t size = 8; };
template <> struct elem_traits<float>   { static constexpr SedsElemKind kind = SEDS_EK_FLOAT;     static constexpr size_t size = 4; };
template <> struct elem_traits<double>  { static constexpr SedsElemKind kind = SEDS_EK_FLOAT;     static constexpr size_t size = 8; };
} // namespace seds_detail

template <typename T>
static inline int seds_router_log(SedsRouter *r,
                                       uint32_t ty,
                                       const T *data,
                                       size_t count,
                                       uint64_t timestamp)
{
    return seds_router_log_typed(r,
                                 ty,
                                 (const void*)data,
                                 count,
                                 seds_detail::elem_traits<T>::size,
                                 seds_detail::elem_traits<T>::kind,
                                 timestamp);
}
#endif // __cplusplus

#ifdef __cplusplus
extern "C" {
#endif

// ---------- C11 _Generic convenience macro ----------
// Usage: seds_router_log_auto(r, ty, ptr, count, ts)
// This selects (kind,size) based on the pointer type.
#if !defined(__cplusplus) && defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
#define SEDS__KIND_SIZE(x, KIND_OUT, SIZE_OUT) do {                                 \
    _Static_assert(sizeof(*(x))==1 || sizeof(*(x))==2 ||                            \
                   sizeof(*(x))==4 || sizeof(*(x))==8,                              \
                   "element size must be 1,2,4,8");                                 \
    enum { SEDS__SIZE = sizeof(*(x)) };                                             \
    (SIZE_OUT) = SEDS__SIZE;                                                        \
    /* Determine kind via _Generic on the dereferenced type */                      \
    (KIND_OUT) =                                                                    \
        _Generic(*(x),                                                              \
            unsigned char: SEDS_EK_UNSIGNED,                                        \
            uint16_t:      SEDS_EK_UNSIGNED,                                        \
            uint32_t:      SEDS_EK_UNSIGNED,                                        \
            uint64_t:      SEDS_EK_UNSIGNED,                                        \
            signed char:   SEDS_EK_SIGNED,                                          \
            int16_t:       SEDS_EK_SIGNED,                                          \
            int32_t:       SEDS_EK_SIGNED,                                          \
            int64_t:       SEDS_EK_SIGNED,                                          \
            float:         SEDS_EK_FLOAT,                                           \
            double:        SEDS_EK_FLOAT,                                           \
            default:       SEDS_EK_UNSIGNED /* sensible default if user passes custom */ \
        );                                                                          \
} while(0)

#define seds_router_log(R, TY, PTR, COUNT, TS)                                 \
    (__extension__({                                                                \
        size_t _esize; SedsElemKind _ekind;                                         \
        SEDS__KIND_SIZE((PTR), _ekind, _esize);                                      \
        seds_router_log_typed((R), (TY), (const void*)(PTR), (COUNT), _esize, _ekind, (TS)); \
    }))
#endif // C11 generic

#ifdef __cplusplus
}
#endif

#endif // SEDSPRINTF_C_H
