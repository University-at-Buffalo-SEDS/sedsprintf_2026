#ifndef SEDSPRINTF_C_H
#define SEDSPRINTF_C_H

#include <stdint.h>
#include <stddef.h>
#include <string.h> /* for strlen in string macros */

#ifdef __cplusplus
extern "C" {

#endif

/* ==============================
   Public enums / constants
   ============================== */

typedef enum SedsDataType
{
    SEDS_DT_TELEMETRY_ERROR = 0,
    SEDS_DT_GPS = 1,
    SEDS_DT_IMU = 2,
    SEDS_DT_BATTERY = 3,
    SEDS_DT_SYSTEM = 4,
    SEDS_DT_BAROMETER = 5,
    SEDS_MESSAGE_DATA = 6
} SedsDataType;

typedef enum SedsDataEndpoint
{
    SEDS_EP_SD = 0,
    SEDS_EP_RADIO = 1,
} SedsDataEndpoint;

typedef enum
{
    SEDS_OK = 0,
    SEDS_ERR = -1,
    SEDS_INVALID_TYPE = -2,
    SEDS_SIZE_MISMATCH = -3,
    SEDS_HANDLER_ERROR = -8,
    SEDS_BAD_ARG = -9,
    SEDS_DESERIALIZE = -10,
} SedsResult;

typedef uint64_t (* SedsNowMsFn)(void * user);

typedef struct SedsRouter SedsRouter;

typedef struct SedsPacketView
{
    uint32_t ty;
    size_t data_size;

    const char * sender;
    size_t sender_len;

    const uint32_t * endpoints;
    size_t num_endpoints;

    uint64_t timestamp;
    const uint8_t * payload;
    size_t payload_len;
} SedsPacketView;

typedef enum SedsElemKind
{
    SEDS_EK_UNSIGNED = 0,
    SEDS_EK_SIGNED = 1,
    SEDS_EK_FLOAT = 2
} SedsElemKind;

typedef SedsResult (* SedsTransmitFn)(const uint8_t * bytes, size_t len, void * user);

typedef SedsResult (* SedsEndpointHandlerFn)(const SedsPacketView * pkt, void * user);

typedef SedsResult (* SedsSerializedHandlerFn)(const uint8_t * bytes, size_t len, void * user);

typedef struct SedsLocalEndpointDesc
{
    uint32_t endpoint;
    SedsEndpointHandlerFn packet_handler; /* optional */
    SedsSerializedHandlerFn serialized_handler; /* optional */
    void * user;
} SedsLocalEndpointDesc;

/* ==============================
   String / error formatting
   ============================== */

int32_t seds_pkt_header_string_len(const SedsPacketView * pkt);

int32_t seds_pkt_to_string_len(const SedsPacketView * pkt);

int32_t seds_error_to_string_len(const int32_t error_code);

SedsResult seds_pkt_header_string(const SedsPacketView * pkt, char * buf, size_t buf_len);

SedsResult seds_pkt_to_string(const SedsPacketView * pkt, char * buf, size_t buf_len);

SedsResult seds_error_to_string(int32_t error_code, char * buf, size_t buf_len);

/* ==============================
   Router lifecycle
   ============================== */

SedsRouter * seds_router_new(SedsTransmitFn tx,
                             void * tx_user,
                             SedsNowMsFn now_ms_cb,
                             const SedsLocalEndpointDesc * handlers,
                             size_t n_handlers);

void seds_router_free(SedsRouter * r);

/* ==============================
   NEW: schema helper
   ============================== */

/**
 * @brief Return the fixed schema payload size (in bytes) required for a type.
 * @return size (>=0) on success; negative SedsResult on error (e.g., invalid type).
 */
int32_t seds_dtype_expected_size(SedsDataType ty);

/* ==============================
   NEW: unified logging entry points
   ============================== */

/**
 * @brief Unified typed logger with optional timestamp + queue flag.
 *
 * If @p timestamp_ms_opt is NULL, the router’s monotonic clock will be used.
 * If @p queue is non-zero, the packet is queued instead of sent immediately.
 *
 * For multi-byte element types, values are encoded in little-endian.
 * The total bytes (count * elem_size) MUST equal the schema size.
 */
SedsResult seds_router_log_typed_ex(SedsRouter * r,
                                    SedsDataType ty,
                                    const void * data,
                                    size_t count,
                                    size_t elem_size,
                                    SedsElemKind elem_kind,
                                    const uint64_t * timestamp_ms_opt, /* NULL => now */
                                    int queue /* 0 = immediate, non-zero = queue */);

/**
 * @brief String/byte logger that pads or truncates to the schema’s size.
 *
 * Copies at most @p len bytes from @p bytes into an internal buffer sized to the
 * schema’s fixed size for @p ty. If @p len < schema, remaining bytes are zeroed.
 * If @p len > schema, input is truncated.
 *
 * This avoids SEDS_SIZE_MISMATCH for fixed-size “message” types while preserving
 * the Router’s size invariants.
 */
SedsResult seds_router_log_string_ex(SedsRouter * r,
                                     SedsDataType ty,
                                     const char * bytes,
                                     size_t len,
                                     const uint64_t * timestamp_ms_opt, /* NULL => now */
                                     int queue /* 0 = immediate, non-zero = queue */);

/* ==============================
   Legacy convenience (still supported)
   ============================== */

SedsResult seds_router_receive_serialized(SedsRouter * r, const uint8_t * bytes, size_t len);

SedsResult seds_router_receive(SedsRouter * r, const SedsPacketView * view);

SedsResult seds_router_process_send_queue(SedsRouter * r);

SedsResult seds_router_queue_tx_message(SedsRouter * r, const SedsPacketView * view);

SedsResult seds_router_process_received_queue(SedsRouter * r);

SedsResult seds_router_rx_serialized_packet_to_queue(SedsRouter * r, const uint8_t * bytes, size_t len);

SedsResult seds_router_rx_packet_to_queue(SedsRouter * r, const SedsPacketView * view);

SedsResult seds_router_process_tx_queue_with_timeout(SedsRouter * r, uint32_t timeout_ms);

SedsResult seds_router_process_rx_queue_with_timeout(SedsRouter * r, uint32_t timeout_ms);

SedsResult seds_router_process_all_queues_with_timeout(SedsRouter * r, uint32_t timeout_ms);

SedsResult seds_router_process_all_queues(SedsRouter * r);

SedsResult seds_router_clear_queues(SedsRouter * r);

SedsResult seds_router_clear_rx_queue(SedsRouter * r);

SedsResult seds_router_clear_tx_queue(SedsRouter * r);

/* ==============================
   Payload extraction / serialization helpers
   ============================== */

const void * seds_pkt_bytes_ptr(const SedsPacketView * pkt, size_t * out_len);

const void * seds_pkt_data_ptr(const SedsPacketView * pkt, size_t elem_size, size_t * out_count);

int32_t seds_pkt_copy_bytes(const SedsPacketView * pkt, void * dst, size_t dst_len);

int32_t seds_pkt_copy_data(const SedsPacketView * pkt, size_t elem_size, void * dst, size_t dst_elems);

SedsResult seds_pkt_get_typed(const SedsPacketView * pkt,
                              void * out,
                              size_t count,
                              size_t elem_size,
                              SedsElemKind elem_kind);

int32_t seds_pkt_serialize_len(const SedsPacketView * view);

int32_t seds_pkt_serialize(const SedsPacketView * view, uint8_t * out, size_t out_len);

typedef struct SedsOwnedPacket SedsOwnedPacket;

SedsOwnedPacket * seds_pkt_deserialize_owned(const uint8_t * bytes, size_t len);

SedsResult seds_owned_pkt_view(const SedsOwnedPacket * pkt, SedsPacketView * out_view);

void seds_owned_pkt_free(SedsOwnedPacket * pkt);

SedsResult seds_pkt_validate_serialized(const uint8_t * bytes, size_t len);

typedef struct SedsOwnedHeader SedsOwnedHeader;

SedsOwnedHeader * seds_pkt_deserialize_header_owned(const uint8_t * bytes, size_t len);

SedsResult seds_owned_header_view(const SedsOwnedHeader * h, SedsPacketView * out_view);

void seds_owned_header_free(SedsOwnedHeader * h);

#ifdef __cplusplus
} /* extern "C" */
#endif

/* ============================================================================
   Header-only helpers for C++ call sites (no extern "C")
   ============================================================================ */
#if defined(__cplusplus)
namespace seds_detail
{
    template < typename
    T >
    struct elem_traits;
    template<>
    struct elem_traits<uint8_t>
    {
        static constexpr SedsElemKind kind = SEDS_EK_UNSIGNED;
        static constexpr size_t size = 1;
    };
    template<>
    struct elem_traits<uint16_t>
    {
        static constexpr SedsElemKind kind = SEDS_EK_UNSIGNED;
        static constexpr size_t size = 2;
    };
    template<>
    struct elem_traits<uint32_t>
    {
        static constexpr SedsElemKind kind = SEDS_EK_UNSIGNED;
        static constexpr size_t size = 4;
    };
    template<>
    struct elem_traits<uint64_t>
    {
        static constexpr SedsElemKind kind = SEDS_EK_UNSIGNED;
        static constexpr size_t size = 8;
    };
    template<>
    struct elem_traits<int8_t>
    {
        static constexpr SedsElemKind kind = SEDS_EK_SIGNED;
        static constexpr size_t size = 1;
    };
    template<>
    struct elem_traits<int16_t>
    {
        static constexpr SedsElemKind kind = SEDS_EK_SIGNED;
        static constexpr size_t size = 2;
    };
    template<>
    struct elem_traits<int32_t>
    {
        static constexpr SedsElemKind kind = SEDS_EK_SIGNED;
        static constexpr size_t size = 4;
    };
    template<>
    struct elem_traits<int64_t>
    {
        static constexpr SedsElemKind kind = SEDS_EK_SIGNED;
        static constexpr size_t size = 8;
    };
    template<>
    struct elem_traits<float>
    {
        static constexpr SedsElemKind kind = SEDS_EK_FLOAT;
        static constexpr size_t size = 4;
    };
    template<>
    struct elem_traits<double>
    {
        static constexpr SedsElemKind kind = SEDS_EK_FLOAT;
        static constexpr size_t size = 8;
    };
}

template<typename T>
static inline SedsResult seds_router_log(SedsRouter * r, SedsDataType ty, const T * data, size_t count)
{
    return seds_router_log_typed_ex(r, ty, data, count,
                                    seds_detail::elem_traits<T>::size,
                                    seds_detail::elem_traits<T>::kind,
                                    /*ts*/NULL, /*queue*/0);
}

template<typename T>
static inline SedsResult seds_router_log_queue(SedsRouter * r, SedsDataType ty, const T * data, size_t count)
{
    return seds_router_log_typed_ex(r, ty, data, count,
                                    seds_detail::elem_traits<T>::size,
                                    seds_detail::elem_traits<T>::kind,
                                    /*ts*/NULL, /*queue*/1);
}

template<typename T>
static inline SedsResult seds_router_log_ts(SedsRouter * r, SedsDataType ty, uint64_t ts_ms, const T * data,
                                            size_t count)
{
    return seds_router_log_typed_ex(r, ty, data, count,
                                    seds_detail::elem_traits<T>::size,
                                    seds_detail::elem_traits<T>::kind,
                                    &ts_ms, /*queue*/0);
}

template<typename T>
static inline SedsResult seds_router_log_queue_ts(SedsRouter * r, SedsDataType ty, uint64_t ts_ms, const T * data,
                                                  size_t count)
{
    return seds_router_log_typed_ex(r, ty, data, count,
                                    seds_detail::elem_traits<T>::size,
                                    seds_detail::elem_traits<T>::kind,
                                    &ts_ms, /*queue*/1);
}

/* String convenience for C++ */
static inline SedsResult seds_router_log_cstr(SedsRouter * r, SedsDataType ty, const char * s)
{
    return seds_router_log_string_ex(r, ty, s, s ? strlen(s) : 0, NULL, 0);
}
static inline SedsResult seds_router_log_cstr_ts(SedsRouter * r, SedsDataType ty, uint64_t ts, const char * s)
{
    return seds_router_log_string_ex(r, ty, s, s ? strlen(s) : 0, &ts, 0);
}
static inline SedsResult seds_router_log_cstr_queue(SedsRouter * r, SedsDataType ty, const char * s)
{
    return seds_router_log_string_ex(r, ty, s, s ? strlen(s) : 0, NULL, 1);
}
static inline SedsResult seds_router_log_cstr_queue_ts(SedsRouter * r, SedsDataType ty, uint64_t ts, const char * s)
{
    return seds_router_log_string_ex(r, ty, s, s ? strlen(s) : 0, &ts, 1);
}

/* Extractor remains the same */
template<typename T>
static inline SedsResult seds_pkt_get(const SedsPacketView * pkt, T * out, size_t count)
{
    return seds_pkt_get_typed(pkt, out, count,
                              seds_detail::elem_traits<T>::size,
                              seds_detail::elem_traits<T>::kind);
}
#endif /* __cplusplus */

/* ============================================================================
   C11 _Generic convenience macros (string-safe)
   ============================================================================ */
#if !defined(__cplusplus) && defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L

#define SEDS__KIND_SIZE(x, KIND_OUT, SIZE_OUT) do {                                    \
    _Static_assert(sizeof(*(x))==1 || sizeof(*(x))==2 ||                                \
                   sizeof(*(x))==4 || sizeof(*(x))==8,                                  \
                   "element size must be 1,2,4,8");                                     \
    (SIZE_OUT) = (size_t)sizeof(*(x));                                                  \
    (KIND_OUT) = _Generic(*(x),                                                         \
        unsigned char: SEDS_EK_UNSIGNED,                                                \
        uint16_t:      SEDS_EK_UNSIGNED,                                                \
        uint32_t:      SEDS_EK_UNSIGNED,                                                \
        uint64_t:      SEDS_EK_UNSIGNED,                                                \
        signed char:   SEDS_EK_SIGNED,                                                  \
        int16_t:       SEDS_EK_SIGNED,                                                  \
        int32_t:       SEDS_EK_SIGNED,                                                  \
        int64_t:       SEDS_EK_SIGNED,                                                  \
        float:         SEDS_EK_FLOAT,                                                   \
        double:        SEDS_EK_FLOAT,                                                   \
        default:       SEDS_EK_UNSIGNED                                                 \
    );                                                                                  \
} while(0)

/* === Generic typed (non-string) shortcuts === */
#define seds_router_log(router, datatype, data, count)                                  \
    (__extension__({                                                                    \
        const void   *_s_data  = (const void*)(data);                                   \
        size_t        _s_count = (size_t)(count);                                       \
        size_t        _s_esize;                                                         \
        SedsElemKind  _s_kind;                                                          \
        SEDS__KIND_SIZE((data), _s_kind, _s_esize);                                      \
        seds_router_log_typed_ex((router), (datatype), _s_data, _s_count,               \
                                 _s_esize, _s_kind, NULL, 0);                            \
    }))

#define seds_router_log_queue(router, datatype, data, count)                            \
    (__extension__({                                                                    \
        const void   *_s_data  = (const void*)(data);                                   \
        size_t        _s_count = (size_t)(count);                                       \
        size_t        _s_esize;                                                         \
        SedsElemKind  _s_kind;                                                          \
        SEDS__KIND_SIZE((data), _s_kind, _s_esize);                                      \
        seds_router_log_typed_ex((router), (datatype), _s_data, _s_count,               \
                                 _s_esize, _s_kind, NULL, 1);                            \
    }))

#define seds_router_log_ts(router, datatype, ts_ms, data, count)                        \
    (__extension__({                                                                    \
        const void   *_s_data  = (const void*)(data);                                   \
        size_t        _s_count = (size_t)(count);                                       \
        size_t        _s_esize;                                                         \
        SedsElemKind  _s_kind;                                                          \
        const uint64_t _s_ts = (uint64_t)(ts_ms);                                       \
        SEDS__KIND_SIZE((data), _s_kind, _s_esize);                                      \
        seds_router_log_typed_ex((router), (datatype), _s_data, _s_count,               \
                                 _s_esize, _s_kind, &_s_ts, 0);                          \
    }))

#define seds_router_log_queue_ts(router, datatype, ts_ms, data, count)                  \
    (__extension__({                                                                    \
        const void   *_s_data  = (const void*)(data);                                   \
        size_t        _s_count = (size_t)(count);                                       \
        size_t        _s_esize;                                                         \
        SedsElemKind  _s_kind;                                                          \
        const uint64_t _s_ts = (uint64_t)(ts_ms);                                       \
        SEDS__KIND_SIZE((data), _s_kind, _s_esize);                                      \
        seds_router_log_typed_ex((router), (datatype), _s_data, _s_count,               \
                                 _s_esize, _s_kind, &_s_ts, 1);                          \
    }))

/* === STRING-SAFE shortcuts (no size mismatch) ===
 * These macros are for NUL-terminated strings (char* / const char*).
 * They compute strlen and call the string-aware EX function which pads/truncates
 * to the schema’s fixed size behind the scenes.
 */
#define seds_router_log_cstr(router, datatype, cstr)                                    \
    (__extension__({                                                                    \
        const char *_s = (const char*)(cstr);                                           \
        seds_router_log_string_ex((router), (datatype), _s, (_s?strlen(_s):0), NULL, 0);\
    }))

#define seds_router_log_cstr_queue(router, datatype, cstr)                              \
    (__extension__({                                                                    \
        const char *_s = (const char*)(cstr);                                           \
        seds_router_log_string_ex((router), (datatype), _s, (_s?strlen(_s):0), NULL, 1);\
    }))

#define seds_router_log_cstr_ts(router, datatype, ts_ms, cstr)                          \
    (__extension__({                                                                    \
        const char *_s = (const char*)(cstr);                                           \
        const uint64_t _s_ts = (uint64_t)(ts_ms);                                       \
        seds_router_log_string_ex((router), (datatype), _s, (_s?strlen(_s):0), &_s_ts, 0);\
    }))

#define seds_router_log_cstr_queue_ts(router, datatype, ts_ms, cstr)                    \
    (__extension__({                                                                    \
        const char *_s = (const char*)(cstr);                                           \
        const uint64_t _s_ts = (uint64_t)(ts_ms);                                       \
        seds_router_log_string_ex((router), (datatype), _s, (_s?strlen(_s):0), &_s_ts, 1);\
    }))

/* Typed extractor unchanged */
#define SEDS__PKT_KIND_SIZE(x, KIND_OUT, SIZE_OUT) SEDS__KIND_SIZE(x, KIND_OUT, SIZE_OUT)

#define seds_pkt_get(pkt, out, count)                                                   \
    (__extension__({                                                                    \
        void        *_s_out   = (void*)(out);                                           \
        size_t       _s_count = (size_t)(count);                                        \
        size_t       _s_esize;                                                          \
        SedsElemKind _s_kind;                                                           \
        SEDS__PKT_KIND_SIZE((out), _s_kind, _s_esize);                                   \
        seds_pkt_get_typed((pkt), _s_out, _s_count, _s_esize, _s_kind);                 \
    }))
#endif /* C11 generic */

#endif /* SEDSPRINTF_C_H */
