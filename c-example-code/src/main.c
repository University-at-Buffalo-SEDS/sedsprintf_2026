#include "telemetry.h"
#include "sedsprintf.h"


int main(void)
{
    init_telemetry_router();
    const float data[3] = {37.7749f, -122.4194f, 30.0f};
    log_telemetry(SEDS_DT_GPS, data, 3);

    return 0;
}
