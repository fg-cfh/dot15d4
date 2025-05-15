use dot15d4_frame3::payload::FramePayload;
use mpmc_channel::HasAddress;

use super::mcps::data::DataIndication;
pub use super::mcps::data::DataRequest;
use super::mlme::beacon::{BeaconNotifyIndication, BeaconRequest};
use super::mlme::set::SetRequestAttribute;

/// Enum representing all (currently) supported MAC services request primitives
pub enum MacRequest {
    /// IEEE 802.15.4-2020, section 8.2.6.4
    MlmeSetRequest(SetRequestAttribute),
    /// IEEE 802.15.4-2020, section 8.2.18.1
    MlmeBeaconRequest(BeaconRequest),
    /// IEEE 802.15.4-2020, section 8.3.2
    McpsDataRequest(DataRequest),
}

impl MacRequest {
    fn new(payload: FramePayload) -> Self {
        Self::McpsDataRequest(DataRequest { payload })
    }
}

/// Dummy implementation to satisfy the generic channel.
///
/// May have to change if we want to direct MLME messages to a different
/// receiver than MCPS messages for example.
impl HasAddress<()> for MacRequest {
    fn address(&self) -> () {
        // no-op
    }
}

pub enum MacIndication {
    McpsData(DataIndication),
    MlmeBeaconNotify(BeaconNotifyIndication),
}

/// Dummy implementation to satisfy the generic channel.
///
/// Will change once we allow several tasks to listen for indications in
/// parallel.
impl HasAddress<()> for MacIndication {
    fn address(&self) -> () {
        // no-op
    }
}
