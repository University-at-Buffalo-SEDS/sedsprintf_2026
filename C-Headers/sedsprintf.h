#ifndef SEDSPRINTF_C_H
#define SEDSPRINTF_C_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {



#endif

// ==============================
// Public enums / constants
// ==============================

/** \brief Telemetry data types (keep in sync with Rust DataType order). */
typedef enum SedsDataType
{
    SEDS_DT_TELEMETRY_ERROR = 0,
    SEDS_DT_GPS = 1,
    SEDS_DT_IMU = 2,
    SEDS_DT_BATTERY = 3,
    SEDS_DT_SYSTEM = 4,
    SEDS_DT_BAROMETER = 5,
    SEDS_IMU_HEX_DATA = 6
} SedsDataType;

/** \brief Telemetry endpoints (keep in sync with Rust DataEndpoint order). */
typedef enum SedsDataEndpoint
{
    SEDS_EP_SD = 0,
    SEDS_EP_RADIO = 1,
} SedsDataEndpoint;

/** \brief Status codes returned by the C API. */
typedef enum
{
    SEDS_OK = 0,
    SEDS_ERR = -1,
    SEDS_BAD_ARG = -2,
    SEDS_INVALID_TYPE = -3,
    SEDS_SIZE_MISMATCH = -4,
    SEDS_DESERIALIZE = -5,
    SEDS_HANDLER_ERROR = -6,
} SedsResult;

/** \brief Opaque Router handle. */
typedef struct SedsRouter SedsRouter;

/** \brief Lightweight view of a telemetry packet (valid only during a callback). */
typedef struct SedsPacketView
{
    uint32_t ty; /**< DataType as u32 */
    size_t data_size; /**< Payload size in bytes */
    const uint32_t * endpoints; /**< Array of DataEndpoint (as u32) */
    size_t num_endpoints;
    uint64_t timestamp; /**< Timestamp units as defined by your system */
    const uint8_t * payload; /**< Raw payload bytes */
    size_t payload_len; /**< == data_size */
} SedsPacketView;

/** \brief Transmit callback: return 0 on success, non-zero on failure. */
typedef SedsResult (* SedsTransmitFn)(const uint8_t * bytes, size_t len, void * user);

/** \brief Local endpoint handler: return 0 on success, non-zero on failure. */
typedef SedsResult (* SedsEndpointHandlerFn)(const SedsPacketView * pkt, void * user);

/** \brief Endpoint handler descriptor (used when constructing a router). */
typedef struct SedsLocalEndpointDesc
{
    uint32_t endpoint; /**< DataEndpoint as u32 */
    SedsEndpointHandlerFn handler; /**< Callback function */
    void * user; /**< Opaque user context passed to callback */
} SedsLocalEndpointDesc;

/**
 * \brief Get required buffer length for seds_pkt_header_string() (includes NUL).
 * \param pkt Pointer to a valid SedsPacketView.
 * \return Required number of bytes (>=1) or negative on error.
 */
SedsResult seds_pkt_header_string_len(const SedsPacketView * pkt);

/**
 * \brief Get required buffer length for seds_pkt_to_string() (includes NUL).
 * \param pkt Pointer to a valid SedsPacketView.
 * \return Required number of bytes (>=1) or negative on error.
 */
SedsResult seds_pkt_to_string_len(const SedsPacketView * pkt);

// ==============================
// Packet -> string formatting (NEW)
// ==============================

/**
 * \brief Write the packet header as text (same format as Rust `packet_header_string`).
 *
 * If \p buf is NULL or \p buf_len == 0, no data is written and the function returns the
 * required buffer length **including** the terminating NUL. If \p buf is non-NULL, up to
 * (\p buf_len - 1) characters are written and the output is always NUL-terminated on success.
 *
 * \param pkt      Pointer to a valid SedsPacketView.
 * \param buf      Destination char buffer (may be NULL to query needed size).
 * \param buf_len  Size of \p buf in bytes (0 allowed to query).
 * \return         Required length including NUL on success (>=1), or a negative error code.
 *                 Possible errors: SEDS_BAD_ARG (-2).
 */
SedsResult seds_pkt_header_string(const SedsPacketView * pkt, char * buf, size_t buf_len);

/**
 * \brief Write the full packet (header + formatted payload) as text
 *        (same format as Rust `packet_to_string`).
 *
 * If \p buf is NULL or \p buf_len == 0, no data is written and the function returns the
 * required buffer length **including** the terminating NUL. If \p buf is non-NULL, up to
 * (\p buf_len - 1) characters are written and the output is always NUL-terminated on success.
 *
 * \param pkt      Pointer to a valid SedsPacketView.
 * \param buf      Destination char buffer (may be NULL to query needed size).
 * \param buf_len  Size of \p buf in bytes (0 allowed to query).
 * \return         Required length including NUL on success (>=1), or a negative error code.
 *                 Possible errors: SEDS_BAD_ARG (-2).
 */
SedsResult seds_pkt_to_string(const SedsPacketView * pkt, char * buf, size_t buf_len);


// ==============================
// Router lifecycle
// ==============================

/**
 * \brief Create a router with an optional transmit callback and an array of local handlers.
 *
 * \param tx         Optional transmit function (NULL if no remote transmit).
 * \param tx_user    Opaque user pointer for \p tx.
 * \param handlers   Array of handler descriptors (may be NULL if n_handlers==0).
 * \param n_handlers Number of entries in \p handlers.
 * \return           New router instance, or NULL on failure.
 */
SedsRouter * seds_router_new(SedsTransmitFn tx,
                             void * tx_user,
                             const SedsLocalEndpointDesc * handlers,
                             size_t n_handlers);

/**
 * \brief Destroy a router created by seds_router_new().
 *
 * \param r Router handle (NULL is safe and a no-op).
 */
void seds_router_free(SedsRouter * r);


// ==============================
// Generic typed logging
// ==============================

/** \brief Element kind used by the generic logging/extraction APIs. */
typedef enum SedsElemKind
{
    SEDS_EK_UNSIGNED = 0, /**< u8/u16/u32/u64/... */
    SEDS_EK_SIGNED = 1, /**< i8/i16/i32/i64/... */
    SEDS_EK_FLOAT = 2 /**< f32/f64 */
} SedsElemKind;

/**
 * \brief Log a typed slice; the router encodes elements to little-endian bytes internally.
 *
 * The total number of bytes (count * elem_size) **must** match the schema size for \p ty.
 * Supported element sizes are 1, 2, 4, and 8 bytes.
 *
 * \param r           Router handle.
 * \param ty          Data type (SedsDataType as u32).
 * \param data        Pointer to first element (may be unaligned).
 * \param count       Number of elements at \p data.
 * \param elem_size   Size of each element in bytes (1,2,4,8).
 * \param elem_kind   Signedness/float kind of the elements.
 * \param timestamp   Timestamp value to attach to the packet.
 * \return            0 on success; negative error code on failure (see status codes).
 */
SedsResult seds_router_log_typed(SedsRouter * r,
                                 SedsDataType ty,
                                 const void * data,
                                 size_t count,
                                 size_t elem_size,
                                 SedsElemKind elem_kind,
                                 uint64_t timestamp);


/**
 * \brief Log a typed slice; the router encodes elements to little-endian bytes internally.
 *
 * The total number of bytes (count * elem_size) **must** match the schema size for \p ty.
 * Supported element sizes are 1, 2, 4, and 8 bytes.
 *
 * \param r           Router handle.
 * \param ty          Data type (SedsDataType as u32).
 * \param data        Pointer to first element (may be unaligned).
 * \param count       Number of elements at \p data.
 * \param elem_size   Size of each element in bytes (1,2,4,8).
 * \param elem_kind   Signedness/float kind of the elements.
 * \param timestamp   Timestamp value to attach to the packet.
 * \return            0 on success; negative error code on failure (see status codes).
 */
SedsResult seds_router_log_queue_typed(SedsRouter * r,
                                       SedsDataType ty,
                                       const void * data,
                                       size_t count,
                                       size_t elem_size,
                                       SedsElemKind elem_kind,
                                       uint64_t timestamp);

// ==============================
// Convenience logging forms
// ==============================

/**
 * \brief Log a raw byte buffer (assumed already in the required little-endian layout).
 *
 * \param r         Router handle.
 * \param ty        Data type (SedsDataType as u32).
 * \param data      Pointer to bytes.
 * \param len       Number of bytes (must equal the schema size for \p ty).
 * \param timestamp Timestamp to attach.
 * \return          0 on success; negative error code on failure.
 */
SedsResult seds_router_log_bytes(SedsRouter * r,
                                 SedsDataType ty,
                                 const uint8_t * data,
                                 size_t len,
                                 uint64_t timestamp);

/**
 * \brief Log an array of f32 values; n_vals*4 must match the schema size for \p ty.
 *
 * \param r         Router handle.
 * \param ty        Data type (SedsDataType as u32).
 * \param vals      Pointer to float elements.
 * \param n_vals    Number of elements at \p vals.
 * \param timestamp Timestamp to attach.
 * \return          0 on success; negative error code on failure.
 */
SedsResult seds_router_log_f32(SedsRouter * r,
                               SedsDataType ty,
                               const float * vals,
                               size_t n_vals,
                               uint64_t timestamp);


// ==============================
// Wire receive / helpers
// ==============================

/**
 * \brief Receive a serialized wire buffer and dispatch to local endpoints.
 *
 * \param r     Router handle.
 * \param bytes Pointer to serialized packet bytes.
 * \param len   Length of \p bytes.
 * \return      0 on success; negative error code on failure.
 */
SedsResult seds_router_receive_serialized(SedsRouter * r,
                                          const uint8_t * bytes,
                                          size_t len);


/**
 * \brief Receive a telemetry buffer and dispatch to local endpoints.
 *
 * \param r     Router handle.
 * \param view   the SedsPacketView to dispatch.
 * \return      0 on success; negative error code on failure.
 */
SedsResult seds_router_receive(SedsRouter * r,
                               SedsPacketView * view);

/**
 * \brief Process and flush all queued transmit packets via the router's send().
 *
 * Pops every pending packet from the internal transmit queue and sends it.
 *
 * \param r Router handle.
 * \return  0 on success; negative error code on failure.
 */
SedsResult seds_router_process_send_queue(SedsRouter * r);

/**
 * \brief Queue a packet for transmission.
 *
 * Validates \p view and pushes it into the router's transmit queue.
 *
 * \param r    Router handle.
 * \param view Pointer to a packet view describing the packet to queue.
 * \return     0 on success; negative error code on failure (e.g., bad args, validation error).
 */
SedsResult seds_router_queue_tx_message(SedsRouter * r,
                                        const SedsPacketView * view);

/* ---------------------- RX (receive) side helpers --------------------- */

/**
 * \brief Process and dispatch all packets currently in the received queue.
 *
 * For each queued packet, this internally serializes it and calls the
 * router's receive path so endpoints are dispatched.
 *
 * \param r Router handle.
 * \return  0 on success; negative error code on failure.
 */
SedsResult seds_router_process_received_queue(SedsRouter * r);

/**
 * \brief Push a serialized wire buffer into the router's received queue.
 *
 * \param r     Router handle.
 * \param bytes Pointer to serialized packet bytes.
 * \param len   Length of \p bytes.
 * \return      0 on success; negative error code on failure (e.g., bad args, parse/validate error).
 */
SedsResult seds_router_rx_serialized_packet_to_queue(SedsRouter * r,
                                                     const uint8_t * bytes,
                                                     size_t len);

/**
 * \brief Push a packet (by view) into the router's received queue.
 *
 * Validates \p view and enqueues it for later processing via
 * seds_router_process_received_queue().
 *
 * \param r    Router handle.
 * \param view Pointer to a packet view describing the packet to queue.
 * \return     0 on success; negative error code on failure (e.g., bad args, validation error).
 */
SedsResult seds_router_rx_packet_to_queue(SedsRouter * r,
                                          const SedsPacketView * view);


// ==============================
// Monotonic clock + timeout processing (new)
// ==============================

/** Monotonic clock callback: must return milliseconds since an arbitrary epoch. */
typedef uint64_t (* SedsNowMsFn)(void * user);

/**
 * Process TX queue until empty or until timeout_ms elapses (wall-clock via SedsNowMsFn).
 *
 * \param r          Router handle.
 * \param now_ms_cb  Clock callback (must not be NULL).
 * \param user       Opaque context passed to now_ms_cb.
 * \param timeout_ms Timeout in milliseconds.
 * \return           0 on success; negative error code on failure.
 */
SedsResult seds_router_process_tx_queue_with_timeout(
    SedsRouter * r,
    SedsNowMsFn now_ms_cb,
    void * user,
    uint32_t timeout_ms
);

/**
 * Process RX queue until empty or until timeout_ms elapses (wall-clock via SedsNowMsFn).
 *
 * \param r          Router handle.
 * \param now_ms_cb  Clock callback (must not be NULL).
 * \param user       Opaque context passed to now_ms_cb.
 * \param timeout_ms Timeout in milliseconds.
 * \return           0 on success; negative error code on failure.
 */
SedsResult seds_router_process_rx_queue_with_timeout(
    SedsRouter * r,
    SedsNowMsFn now_ms_cb,
    void * user,
    uint32_t timeout_ms
);

/**
 * Process all queues until empty or until timeout_ms elapses (wall-clock via SedsNowMsFn).
 *
 * \param r          Router handle.
 * \param now_ms_cb  Clock callback (must not be NULL).
 * \param user       Opaque context passed to now_ms_cb.
 * \param timeout_ms Timeout in milliseconds.
 * \return           0 on success; negative error code on failure.
 */
SedsResult seds_router_process_all_queues_with_timeout(
    SedsRouter * r,
    SedsNowMsFn now_ms_cb,
    void * user,
    uint32_t timeout_ms
);


// ==============================
// Queue utilities (new)
// ==============================

/** Process both queues (TX then RX). */
SedsResult seds_router_process_all_queues(SedsRouter * r);

/** Clear both TX and RX queues. */
SedsResult seds_router_clear_queues(SedsRouter * r);

/** Clear only the RX queue. */
SedsResult seds_router_clear_rx_queue(SedsRouter * r);

/** Clear only the TX queue. */
SedsResult seds_router_clear_tx_queue(SedsRouter * r);

/**
 * \brief Decode a packet view's payload into f32 values.
 *
 * \param pkt Pointer to a valid SedsPacketView (from a callback).
 * \param out Output buffer for \p n floats.
 * \param n   Number of floats to decode; must satisfy n*4 == pkt->payload_len.
 * \return    0 on success; negative error code on failure.
 */
SedsResult seds_pkt_get_f32(const SedsPacketView * pkt, float * out, size_t n);

/**
 * \brief Generic typed extractor (mirror of seds_router_log_typed).
 *
 * Reads \p count elements from \p pkt->payload as little-endian values of the
 * given (elem_kind, elem_size) and writes them to \p out in host endianness.
 * \p out may be unaligned. Supported elem_size values: 1,2,4,8.
 *
 * \param pkt        Pointer to a valid SedsPacketView (from a callback).
 * \param out        Destination buffer (array of elements).
 * \param count      Number of elements to decode.
 * \param elem_size  Size of each element in bytes (1,2,4,8).
 * \param elem_kind  Element kind (SEDS_EK_UNSIGNED, SEDS_EK_SIGNED, SEDS_EK_FLOAT).
 * \return           0 on success; negative error code on failure.
 */
SedsResult seds_pkt_get_typed(const SedsPacketView * pkt,
                              void * out,
                              size_t count,
                              size_t elem_size,
                              SedsElemKind elem_kind);

#ifdef __cplusplus
} // extern "C"
#endif


// ============================================================================
// Header-only helpers for nicer call sites
// ============================================================================

// ---------- C++ templates ----------
#ifdef __cplusplus
extern "C" {



#endif

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
} // namespace seds_detail

/**
 * \brief C++ convenience wrapper that deduces element kind/size from \p T.
 *
 * \tparam T         Element type (uint8_t/16/32/64, int8_t/16/32/64, float, double).
 * \param router     Router handle.
 * \param datatype   Data type (SedsDataType as u32).
 * \param data       Pointer to first element.
 * \param count      Number of elements.
 * \param timestamp  Timestamp to attach.
 * \return           0 on success; negative error code on failure.
 */
template<typename T>
static inline SedsResult seds_router(SedsRouter * router,
                                     SedsDataType datatype,
                                     const T * data,
                                     size_t count,
                                     uint64_t timestamp)
{
    return seds_router_log_typed(router,
                                 datatype,
                                 (const void *) data,
                                 count,
                                 seds_detail::elem_traits<T>::size,
                                 seds_detail::elem_traits<T>::kind,
                                 timestamp);
}

/**
 * \brief C++ convenience wrapper that deduces element kind/size from \p T.
 *
 * \tparam T         Element type (uint8_t/16/32/64, int8_t/16/32/64, float, double).
 * \param router     Router handle.
 * \param datatype   Data type (SedsDataType as u32).
 * \param data       Pointer to first element.
 * \param count      Number of elements.
 * \param timestamp  Timestamp to attach.
 * \return           0 on success; negative error code on failure.
 */
template<typename T>
static inline SedsResult seds_router_queue(SedsRouter * router,
                                           SedsDataType datatype,
                                           const T * data,
                                           size_t count,
                                           uint64_t timestamp)
{
    return seds_router_log_queue_typed(router,
                                       datatype,
                                       (const void *) data,
                                       count,
                                       seds_detail::elem_traits<T>::size,
                                       seds_detail::elem_traits<T>::kind,
                                       timestamp);
}


/**
 * \brief C++ convenience extractor that deduces element kind/size from \p T.
 *
 * \tparam T   Element type (uint8_t/16/32/64, int8_t/16/32/64, float, double).
 * \param pkt  Pointer to valid SedsPacketView.
 * \param out  Destination buffer for \p count elements of \p T.
 * \param count Number of elements to decode.
 * \return     0 on success; negative error code on failure.
 */
template<typename T>
static inline SedsResult seds_pkt_get(const SedsPacketView * pkt, T * out, size_t count)
{
    return seds_pkt_get_typed(pkt,
                              (void *) out,
                              count,
                              seds_detail::elem_traits<T>::size,
                              seds_detail::elem_traits<T>::kind);
}
#endif // __cplusplus

#ifdef __cplusplus
} // extern "C"
#endif


// ---------- C11 _Generic convenience macros ----------
#if !defined(__cplusplus) && defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L

/**
 * \brief Internal: infer (kind,size) from pointer expression \p x.
 *
 * \param x        Pointer expression to element type.
 * \param KIND_OUT Lvalue of type SedsElemKind to receive the kind.
 * \param SIZE_OUT Lvalue of type size_t to receive the size in bytes.
 */
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
        default:       SEDS_EK_UNSIGNED /* fallback for custom integer-like data */     \
    );                                                                                  \
} while(0)

/**
 * \brief C11 convenience wrapper that deduces element kind/size from \p data.
 *
 * \param router     SedsRouter*
 * \param datatype   Data type (SedsDataType as u32)
 * \param data       Pointer to first element (e.g., uint16_t*, float*, ...)
 * \param count      Number of elements
 * \param timestamp  Timestamp to attach
 * \return          0 on success; negative error code on failure.
 */
#define seds_router_log(router, datatype, data, count, timestamp)                       \
    (__extension__({                                                                    \
        const void   *_seds_data  = (const void*)(data);                                \
        size_t        _seds_count = (size_t)(count);                                    \
        size_t        _seds_esize;                                                      \
        SedsElemKind  _seds_kind;                                                       \
        SEDS__KIND_SIZE((data), _seds_kind, _seds_esize);                               \
        seds_router_log_typed((router), (datatype), _seds_data, _seds_count,            \
                              _seds_esize, _seds_kind, (timestamp));                    \
    }))


/**
 * \brief C11 convenience wrapper that deduces element kind/size from \p data.
 *
 * \param router     SedsRouter*
 * \param datatype   Data type (SedsDataType as u32)
 * \param data       Pointer to first element (e.g., uint16_t*, float*, ...)
 * \param count      Number of elements
 * \param timestamp  Timestamp to attach
 * \return          0 on success; negative error code on failure.
 */
#define seds_router_log_queue(router, datatype, data, count, timestamp)                 \
    (__extension__({                                                                    \
        const void   *_seds_data  = (const void*)(data);                                \
        size_t        _seds_count = (size_t)(count);                                    \
        size_t        _seds_esize;                                                      \
        SedsElemKind  _seds_kind;                                                       \
        SEDS__KIND_SIZE((data), _seds_kind, _seds_esize);                               \
        seds_router_log_queue_typed((router), (datatype), _seds_data, _seds_count,      \
                                    _seds_esize, _seds_kind, (timestamp));              \
    }))

/**
 * \brief C11 convenience extractor that deduces element kind/size from \p out.
 *
 * Example:
 * \code
 *   float vals[16];
 *   seds_pkt_get(pkt, vals, 16);
 * \endcode
 *
 * \param pkt    Pointer to SedsPacketView.
 * \param out    Destination pointer (type drives kind/size deduction).
 * \param count  Number of elements to decode.
 * \return       0 on success; negative error code on failure.
 */
#define SEDS__PKT_KIND_SIZE(x, KIND_OUT, SIZE_OUT) SEDS__KIND_SIZE(x, KIND_OUT, SIZE_OUT)

#define seds_pkt_get(pkt, out, count)                                             \
    (__extension__({                                                                    \
        void       *_s_out     = (void*)(out);                                          \
        size_t      _s_count   = (size_t)(count);                                       \
        size_t      _s_esize;                                                           \
        SedsElemKind _s_kind;                                                           \
        SEDS__PKT_KIND_SIZE((out), _s_kind, _s_esize);                                  \
        seds_pkt_get_typed((pkt), _s_out, _s_count, _s_esize, _s_kind);                 \
    }))

#endif // C11 generic

#endif // SEDSPRINTF_C_H
