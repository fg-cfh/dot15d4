use dot15d4_frame3::{driver::DriverConfig, mpdu::MpduFrame, payload::FramePayload};
use rand_core::RngCore;

use super::MacService;

pub struct BeaconRequest {}

pub struct BeaconConfirm {}

pub struct BeaconNotifyIndication {
    /// The received frame payload
    pub payload: FramePayload,
    /// Timestamp of frame reception
    pub timestamp: u32,
}

#[allow(dead_code)]
impl<'svc, Rng: RngCore, TIMER, Config: DriverConfig> MacService<'svc, Rng, TIMER, Config> {
    /// Requests the generation of a Beacon frame or Enhanced Beacon frame.
    pub(crate) async fn mlme_beacon_request(
        &self,
        _request: &BeaconRequest,
    ) -> Result<BeaconConfirm, ()> {
        // TODO: fill with correct values
        let frame_repr = FrameBuilder::new_beacon_request()
            .finalize()
            .expect("A simple beacon request should always be possible to build");
        frame_repr.emit(&mut DataFrame::new_unchecked(&mut tx.buffer));
        self.phy_send(tx).await;

        Err(())
    }

    pub(crate) async fn mlme_beacon_notify_indication(&self, _mpdu: MpduFrame) {
        // TODO: support Beacon Notify indication
        info!("Received Beacon Notification");
    }
}
