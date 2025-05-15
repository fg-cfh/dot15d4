use dot15d4_frame3::driver::DriverConfig;
use embedded_hal_async::delay::DelayNs;

use crate::mac::MacService;
use rand_core::RngCore;

#[allow(dead_code)]
pub struct PurgeConfirm {
    msdu_handle: u8,
}

pub enum PurgeError {
    InvalidHandle,
}

#[allow(dead_code)]
impl<'svc, Rng: RngCore, TIMER: DelayNs + Clone, Config: DriverConfig>
    MacService<'svc, Rng, TIMER, Config>
{
    /// Allows a higher layer to purge an MSDU from the transaction
    /// queue.
    async fn purge_request(&mut self) -> Result<PurgeConfirm, PurgeError> {
        // TODO: not supported
        Err(PurgeError::InvalidHandle)
    }
}
