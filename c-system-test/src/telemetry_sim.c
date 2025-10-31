// telemetry_sim_threadsafe.c
#include "telemetry_sim.h"
#include <stdlib.h>
#include <stdio.h>
#include <sys/time.h>
#include <stdint.h>

#ifdef _WIN32
  #define WIN32_LEAN_AND_MEAN
  #include <windows.h>
#else
  #include <pthread.h>
#endif

/* ===================== Global recursive lock ===================== */

#ifdef _WIN32
static INIT_ONCE        g_once   = INIT_ONCE_STATIC_INIT;
static CRITICAL_SECTION g_cs;

static BOOL CALLBACK win_init_mutex(PINIT_ONCE once, PVOID param, PVOID *ctx) {
    (void)once; (void)param; (void)ctx;
    InitializeCriticalSection(&g_cs); // recursive by design
    return TRUE;
}
static inline void lock_init_once(void) { InitOnceExecuteOnce(&g_once, win_init_mutex, NULL, NULL); }
static inline void LOCK(void)   { EnterCriticalSection(&g_cs); }
static inline void UNLOCK(void) { LeaveCriticalSection(&g_cs); }

#else
static pthread_mutex_t g_mtx;
static pthread_once_t  g_once = PTHREAD_ONCE_INIT;

static void posix_init_mutex(void) {
    pthread_mutexattr_t attr;
    pthread_mutexattr_init(&attr);
    pthread_mutexattr_settype(&attr, PTHREAD_MUTEX_RECURSIVE);
    pthread_mutex_init(&g_mtx, &attr);
    pthread_mutexattr_destroy(&attr);
}
static inline void lock_init_once(void) { pthread_once(&g_once, posix_init_mutex); }
static inline void LOCK(void)   { pthread_mutex_lock(&g_mtx); }
static inline void UNLOCK(void) { pthread_mutex_unlock(&g_mtx); }
#endif

/* ===================== Time ===================== */

// Monotonic milliseconds (used by nodes + bus t0)
uint64_t host_now_ms(const void * user)
{
    (void) user;
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

/* ===================== BUS ===================== */

void bus_init(SimBus * bus)
{
    if (!bus) return;
    lock_init_once();
    LOCK();
    bus->nodes  = NULL;
    bus->count  = 0;
    bus->cap    = 0;
    bus->t0_ms  = host_now_ms(NULL); // zero point
    UNLOCK();
}

void bus_free(SimBus * bus)
{
    if (!bus) return;
    lock_init_once();
    LOCK();
    free(bus->nodes);
    bus->nodes = NULL;
    bus->count = 0;
    bus->cap   = 0;
    UNLOCK();
}

size_t bus_register(SimBus * bus, SimNode * n)
{
    if (!bus || !n) return (size_t)-1;
    lock_init_once();
    LOCK();

    if (bus->count == bus->cap)
    {
        size_t newcap = bus->cap ? bus->cap * 2 : 4;
        SimNode ** tmp = (SimNode**)realloc(bus->nodes, newcap * sizeof(*tmp));
        if (!tmp)
        {
            fprintf(stderr, "bus_register: OOM\n");
            UNLOCK();
            return (size_t)-1;
        }
        bus->nodes = tmp;
        bus->cap   = newcap;
    }
    bus->nodes[bus->count] = n;
    size_t id = bus->count++;
    UNLOCK();
    return id;
}

SedsResult bus_send(SimBus * bus, const SimNode * from, const uint8_t * bytes, size_t len)
{
    if (!bus || !bytes || !len) return SEDS_ERR;
    lock_init_once();
    LOCK();

    // Broadcast to all nodes except sender
    for (size_t i = 0; i < bus->count; ++i)
    {
        SimNode * n = bus->nodes[i];
        if (n == from) continue;
        // node_rx takes the same recursive lock; safe to reenter
        node_rx(n, bytes, len);
    }

    UNLOCK();
    return SEDS_OK;
}

/* ===================== NODE ===================== */

// TX for a node: push raw bytes onto the bus.
static SedsResult node_tx_send(const uint8_t * bytes, const size_t len, void * user)
{
    SimNode * self = (SimNode*)user;
    if (!self || !self->bus) return SEDS_ERR;
    // bus_send does its own locking
    return bus_send(self->bus, self, bytes, len);
}

SedsResult radio_handler_serial(const uint8_t *bytes, const size_t len, void *user)
{
    (void)len;
    SimNode * self = (SimNode*)user;

    // Deserialize into owned packet
    SedsOwnedPacket *owned = seds_pkt_deserialize_owned(bytes, len);
    if (!owned) {
        fprintf(stderr, "[RADIO] deserialize failed\n");
        return SEDS_ERR;
    }

    SedsPacketView view;
    if (seds_owned_pkt_view(owned, &view) != SEDS_OK) {
        fprintf(stderr, "[RADIO] owned_pkt_view failed\n");
        seds_owned_pkt_free(owned);
        return SEDS_ERR;
    }

    char buf[seds_pkt_to_string_len(&view)];
    SedsResult s = seds_pkt_to_string(&view, buf, sizeof(buf));
    if (s != SEDS_OK) {
        fprintf(stderr, "[RADIO] to_string failed: %d\n", s);
        seds_owned_pkt_free(owned);
        return s;
    }

    lock_init_once();
    LOCK();
    if (self) self->radio_hits++;
    printf("[RADIO] %s\n", buf);
    UNLOCK();

    seds_owned_pkt_free(owned);
    return SEDS_OK;
}

SedsResult sdcard_handler(const SedsPacketView * pkt, void * user)
{
    SimNode * self = (SimNode*)user;
    char buf[seds_pkt_to_string_len(pkt)];
    const SedsResult s = seds_pkt_to_string(pkt, buf, sizeof(buf));
    if (s != SEDS_OK)
    {
        fprintf(stderr, "[SD] to_string failed: %d\n", s);
        return s;
    }

    lock_init_once();
    LOCK();
    if (self) self->sd_hits++;
    printf("[SD] wrote: %s\n", buf);
    UNLOCK();

    return SEDS_OK;
}

uint64_t node_now_since_bus_ms(void *user)
{
    const SimNode *self = (const SimNode*)user;
    const uint64_t now  = host_now_ms(NULL);
    // No need to lock for read-only snapshot here; t0_ms set at init and never changes
    return (self && self->bus) ? (now - self->bus->t0_ms) : 0;
}

SedsResult node_init(SimNode * n, SimBus * bus, const char * name, int radio, int sdcard)
{
    if (!n || !bus) return SEDS_ERR;

    lock_init_once();
    LOCK();

    n->r          = NULL;
    n->bus        = bus;
    n->name       = name ? name : "node";
    n->has_radio  = radio ? 1 : 0;
    n->has_sdcard = sdcard ? 1 : 0;

    n->radio_hits = 0;
    n->sd_hits    = 0;

    SedsLocalEndpointDesc locals[2];
    uint32_t num = 0;

    if (n->has_radio)
    {
        locals[num++] = (SedsLocalEndpointDesc){
            .endpoint = SEDS_EP_GROUND_STATION,
            .packet_handler = radio_handler,
            .user = (void *) n
        };
    }
    if (n->has_sdcard)
    {
        locals[num++] = (SedsLocalEndpointDesc){
            .endpoint       = SEDS_EP_SD_CARD, // your enum
            .packet_handler = sdcard_handler,
            .user           = (void*)n
        };
    }

    n->r = seds_router_new(
        node_tx_send,
        n, // tx_user
        node_now_since_bus_ms,
        (num ? locals : NULL),
        num
    );
    if (!n->r)
    {
        fprintf(stderr, "[%s] Failed to create router\n", n->name);
        UNLOCK();
        return SEDS_ERR;
    }

    bus_register(bus, n);
    UNLOCK();
    return SEDS_OK;
}

void node_free(SimNode * n)
{
    if (!n) return;
    lock_init_once();
    LOCK();
    if (n->r)
    {
        seds_router_free(n->r);
        n->r = NULL;
    }
    n->bus = NULL;
    UNLOCK();
}

void node_rx(SimNode * n, const uint8_t * bytes, const size_t len)
{
    if (!n || !n->r || !bytes || !len) return;
    lock_init_once();
    LOCK();
    seds_router_rx_serialized_packet_to_queue(n->r, bytes, len);
    UNLOCK();
}

SedsResult node_log(
    SimNode * n,
    SedsDataType data_type,
    const void * data,
    size_t element_count,
    size_t element_size)
{
    if (!n || !n->r || !data || element_count == 0 || element_size == 0) return SEDS_ERR;

    const size_t total_bytes = element_count * element_size;
    lock_init_once();
    LOCK();
    SedsResult r = seds_router_log_queue(n->r, data_type, data, total_bytes);
    UNLOCK();
    return r;
}
