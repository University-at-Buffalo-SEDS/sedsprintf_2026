// telemetry_host_threadsafe.c
#include "telemetry.h"
#include "sedsprintf.h"

#include <stdio.h>
#include <stdint.h>
#include <time.h>
#include <sys/time.h>

#ifdef _WIN32
  #define WIN32_LEAN_AND_MEAN
  #include <windows.h>
#else
  #include <pthread.h>
#endif

/* ---------------- Monotonic ms provider ---------------- */
static uint64_t host_now_ms(void *user) {
    (void)user;
#ifdef _WIN32
    static LARGE_INTEGER freq = {0};
    LARGE_INTEGER ctr;
    if (freq.QuadPart == 0) { QueryPerformanceFrequency(&freq); }
    QueryPerformanceCounter(&ctr);
    return (uint64_t)((ctr.QuadPart * 1000ULL) / (uint64_t)freq.QuadPart);
#else
    struct timeval tv;
    gettimeofday(&tv, NULL);
    return (uint64_t)tv.tv_sec * 1000ULL + (uint64_t)(tv.tv_usec / 1000ULL);
#endif
}

/* ---------------- Global router state ---------------- */
RouterState g_router = { .r = NULL, .created = 0, .start_time = 0 };

/* ---------------- Internal locking (recursive) ---------------- */
#ifdef _WIN32
static INIT_ONCE        g_once   = INIT_ONCE_STATIC_INIT;
static CRITICAL_SECTION g_cs;

static BOOL CALLBACK win_init_mutex(PINIT_ONCE once, PVOID param, PVOID *ctx) {
    (void)once; (void)param; (void)ctx;
    InitializeCriticalSection(&g_cs);  // recursive by design
    return TRUE;
}
static inline void lock_init_once(void) { InitOnceExecuteOnce(&g_once, win_init_mutex, NULL, NULL); }
static inline void LOCK(void)   { EnterCriticalSection(&g_cs); }
static inline void UNLOCK(void) { LeaveCriticalSection(&g_cs); }

#else
static pthread_mutex_t   g_mtx;
static pthread_once_t    g_once = PTHREAD_ONCE_INIT;

static void posix_init_mutex(void) {
    pthread_mutexattr_t attr;
    pthread_mutexattr_init(&attr);
    pthread_mutexattr_settype(&attr, PTHREAD_MUTEX_RECURSIVE);
    pthread_mutex_init(&g_mtx, &attr);
    pthread_mutexattr_destroy(&attr);
}
static void lock_init_once(void) { pthread_once(&g_once, posix_init_mutex); }
static void LOCK(void)   { pthread_mutex_lock(&g_mtx); }
static void UNLOCK(void) { pthread_mutex_unlock(&g_mtx); }
#endif

/* ---------------- TX: stub ---------------- */
SedsResult tx_send(const uint8_t *bytes, size_t len, void *user) {
    (void)user; (void)bytes; (void)len;
    return SEDS_OK;
}

/* ---------------- RX helpers ---------------- */
void rx_synchronous(const uint8_t *bytes, size_t len) {
    if (!bytes || len == 0) return;
    lock_init_once();
    if (!g_router.r) { if (init_telemetry_router() != SEDS_OK) return; }
    LOCK();
    seds_router_receive_serialized(g_router.r, bytes, len);
    UNLOCK();
}

void rx_asynchronous(const uint8_t *bytes, size_t len) {
    if (!bytes || len == 0) return;
    lock_init_once();
    if (!g_router.r) { if (init_telemetry_router() != SEDS_OK) return; }
    LOCK();
    seds_router_rx_serialized_packet_to_queue(g_router.r, bytes, len);
    UNLOCK();
}

/* ---------------- Local endpoint handler ---------------- */
SedsResult on_radio_packet(const SedsPacketView *pkt, void *user) {
    (void)user;
    char buf[seds_pkt_to_string_len(pkt)];
    SedsResult s = seds_pkt_to_string(pkt, buf, sizeof(buf));
    if (s != SEDS_OK) {
        printf("on_radio_packet: seds_pkt_to_string failed: %d\n", s);
        return s;
    }
    printf("on_radio_packet: %s\n", buf);
    return SEDS_OK;
}

/* ---------------- Router init (idempotent + locked) ---------------- */
SedsResult init_telemetry_router(void) {
    lock_init_once();

    /* Fast path without lock */
    if (g_router.created && g_router.r) return SEDS_OK;

    LOCK();
    if (g_router.created && g_router.r) { UNLOCK(); return SEDS_OK; }

    const SedsLocalEndpointDesc locals[] = {
        { .endpoint = SEDS_EP_RADIO, .packet_handler = on_radio_packet, .user = NULL },
    };

    SedsRouter *r = seds_router_new(
        tx_send,
        NULL,           /* tx_user */
        host_now_ms,    /* clock */
        locals,
        sizeof(locals) / sizeof(locals[0])
    );

    if (!r) {
        printf("Error: failed to create router\n");
        g_router.r = NULL;
        g_router.created = 0;
        UNLOCK();
        return SEDS_ERR;
    }

    g_router.r          = r;
    g_router.created    = 1;
    g_router.start_time = host_now_ms(NULL);

    UNLOCK();
    return SEDS_OK;
}

/* ---------------- Logging APIs (locked) ---------------- */
SedsResult log_telemetry_synchronous(SedsDataType data_type,
                                     const void *data,
                                     size_t element_count,
                                     size_t element_size) {
    if (!data || element_count == 0 || element_size == 0) return SEDS_ERR;
    lock_init_once();
    if (!g_router.r) { if (init_telemetry_router() != SEDS_OK) return SEDS_ERR; }

    const size_t total_bytes = element_count * element_size;
    LOCK();
    const SedsResult res = seds_router_log(g_router.r, data_type, data, total_bytes);
    UNLOCK();
    return res;
}

SedsResult log_telemetry_asynchronous(SedsDataType data_type,
                                      const void *data,
                                      size_t element_count,
                                      size_t element_size) {
    if (!data || element_count == 0 || element_size == 0) return SEDS_ERR;
    lock_init_once();
    if (!g_router.r) { if (init_telemetry_router() != SEDS_OK) return SEDS_ERR; }

    const size_t total_bytes = element_count * element_size;
    LOCK();
    const SedsResult res = seds_router_log_queue(g_router.r, data_type, data, total_bytes);
    UNLOCK();
    return res;
}

/* ---------------- Queue processing (locked) ---------------- */
SedsResult dispatch_tx_queue(void) {
    lock_init_once();
    if (!g_router.r) { if (init_telemetry_router() != SEDS_OK) return SEDS_ERR; }
    LOCK();
    const SedsResult res = seds_router_process_send_queue(g_router.r);
    UNLOCK();
    return res;
}

SedsResult process_rx_queue(void) {
    lock_init_once();
    if (!g_router.r) { if (init_telemetry_router() != SEDS_OK) return SEDS_ERR; }
    LOCK();
    const SedsResult res = seds_router_process_received_queue(g_router.r);
    UNLOCK();
    return res;
}

SedsResult dispatch_tx_queue_timeout(uint32_t timeout_ms) {
    lock_init_once();
    if (!g_router.r) { if (init_telemetry_router() != SEDS_OK) return SEDS_ERR; }
    LOCK();
    const SedsResult res = seds_router_process_tx_queue_with_timeout(g_router.r, timeout_ms);
    UNLOCK();
    return res;
}

SedsResult process_rx_queue_timeout(uint32_t timeout_ms) {
    lock_init_once();
    if (!g_router.r) { if (init_telemetry_router() != SEDS_OK) return SEDS_ERR; }
    LOCK();
    const SedsResult res = seds_router_process_rx_queue_with_timeout(g_router.r, timeout_ms);
    UNLOCK();
    return res;
}

SedsResult process_all_queues_timeout(uint32_t timeout_ms) {
    lock_init_once();
    if (!g_router.r) { if (init_telemetry_router() != SEDS_OK) return SEDS_ERR; }
    LOCK();
    const SedsResult res = seds_router_process_all_queues_with_timeout(g_router.r, timeout_ms);
    UNLOCK();
    return res;
}

/* ---------------- Error printing ---------------- */
SedsResult print_handle_telemetry_error(const int32_t error_code) {
    char buf[seds_error_to_string_len(error_code)];
    const SedsResult res = seds_error_to_string(error_code, buf, sizeof(buf));
    if (res != SEDS_OK) {
        printf("handle_error: seds_error_to_string failed: %d\n", res);
        return res;
    }
    printf("Error: %s\n", buf);
    return SEDS_OK;
}
