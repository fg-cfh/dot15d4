use dot15d4_frame3::driver::DriverConfig;
use embedded_hal_async::delay::DelayNs;
use rand_core::RngCore;

use crate::mac::MacService;

pub struct AssociateConfirm;

#[allow(dead_code)]
impl<'svc, Rng: RngCore, TIMER: DelayNs + Clone, Config: DriverConfig>
    MacService<'svc, Rng, TIMER, Config>
{
    /// Requests the association with a coordinator.
    async fn mlme_associate_request(&self) -> Result<AssociateConfirm, ()> {
        // TODO: support association
        Err(())
    }
}
