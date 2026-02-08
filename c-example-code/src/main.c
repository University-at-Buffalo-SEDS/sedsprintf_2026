#include "telemetry.h"
#include "sedsprintf.h"
#include <unistd.h> // for usleep


int main(void)
{
    //synchronous code
    init_telemetry_router();
    const float data[3] = {37.7749f, -122.4194f, 30.0f};
    SedsResult result = log_telemetry_synchronous(SEDS_DT_GPS_DATA, data, 3, sizeof(data[0]));
    if (result != SEDS_OK)
    {
        print_handle_telemetry_error(result);
    }
    const uint64_t announce[2] = {10ULL, 1000ULL};
    result = log_telemetry_synchronous(SEDS_DT_TIME_SYNC_ANNOUNCE, announce, 2, sizeof(announce[0]));
    if (result != SEDS_OK)
    {
        print_handle_telemetry_error(result);
    }
    usleep(1000); //simulate some delay
    //asynchronous code
    //this would be in send routine of the data collector
    result = log_telemetry_asynchronous(SEDS_DT_GPS_DATA, data, 3, sizeof(data[0]));
    if (result != SEDS_OK)
    {
        print_handle_telemetry_error(result);
    }
    const uint64_t req[2] = {1ULL, 1100ULL};
    result = log_telemetry_asynchronous(SEDS_DT_TIME_SYNC_REQUEST, req, 2, sizeof(req[0]));
    if (result != SEDS_OK)
    {
        print_handle_telemetry_error(result);
    }
    //this would be in the main loop of the program or in a freertos task.
    result = process_all_queues_timeout(20);
    if (result != SEDS_OK)
    {
        print_handle_telemetry_error(result);
    }
    // dispatch_tx_queue_timeout(100);

    //this would be in the isr of the receiver
    //rx_asynchronous(received_bytes, received_length);

    //


    return 0;
}
