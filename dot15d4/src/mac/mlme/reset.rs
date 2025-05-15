use core::cell::RefCell;

use crate::mac::pib;
use dot15d4_frame3::driver::DriverConfig;
use embedded_hal_async::delay::DelayNs;
use rand_core::RngCore;

use super::MacService;

#[allow(dead_code)]
pub struct ResetConfirm {
    status: bool,
}

#[allow(dead_code)]
impl<'svc, Rng: RngCore, TIMER: DelayNs + Clone, Config: DriverConfig>
    MacService<'svc, Rng, TIMER, Config>
{
    /// Used by the next higher layer to request a reset operation that
    /// involves resetting the PAN Information Base
    async fn mlme_reset_request(&mut self, set_default_pib: bool) -> Result<ResetConfirm, ()> {
        if set_default_pib {
            self.pib = RefCell::new(pib::Pib::default());
        }
        Ok(ResetConfirm { status: true })
    }
}
