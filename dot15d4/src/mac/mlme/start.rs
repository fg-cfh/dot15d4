use dot15d4_frame3::driver::DriverConfig;
use embedded_hal_async::delay::DelayNs;
use rand_core::RngCore;

use super::MacService;

struct StartConfirm {}

#[allow(dead_code)]
impl<'svc, Rng: RngCore, TIMER: DelayNs + Clone, Config: DriverConfig>
    MacService<'svc, Rng, TIMER, Config>
{
    /// Used by PAN coordinator to initiate a new PAN or to begin using a new
    /// configuration. Also used by a device already associated with an
    /// existing PAN to begin using a new configuration.
    async fn mlme_start_request(&mut self) -> Result<StartConfirm, ()> {
        Err(())
    }
}
