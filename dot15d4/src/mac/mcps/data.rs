use crate::mac::{MacIndication, MacService};
use dot15d4_frame3::{driver::DriverConfig, mpdu::MpduFrame, payload::FramePayload};
use embedded_hal_async::delay::DelayNs;
use rand_core::RngCore;

pub enum DataError {
    // TODO: not supported
    TransactionOverflow,
    // TODO: not supported
    TransactionExpired,
    // TODO: not supported
    ChannelAccessFailure,
    // TODO: not supported
    InvalidAddress,
    // TODO: not supported
    NoAck,
    // TODO: not supported
    CounterError,
    // TODO: not supported
    FrameTooLong,
    // TODO: not supported
    InvalidParameter,
}

pub struct DataRequest {
    /// The payload to be sent.
    pub payload: FramePayload,
}

pub struct DataConfirm {
    /// Timestamp of frame transmission
    pub timestamp: u32,
    /// Whether the frame has been acknowledged or not
    pub acked: bool,
}

pub struct DataIndication {
    /// The received payload.
    pub payload: FramePayload,
    /// Timestamp of frame reception
    pub timestamp: u32,
}

impl<'svc, Rng: RngCore, TIMER: DelayNs + Clone, Config: DriverConfig>
    MacService<'svc, Rng, TIMER, Config>
{
    /// Requests the transfer of data to another device
    pub async fn mcps_data_request(
        &self,
        data_request: DataRequest,
    ) -> Result<DataConfirm, DataError> {
        let mut mpdu = data_request.payload.into_mpdu_frame();
        let sequence_number = Self::set_ack(&mut mpdu);

        self.radio_send(mpdu.into_radio_frame()).await;
        let acked = match sequence_number {
            Some(sequence_number) => self.wait_for_ack(sequence_number).await,
            _ => true,
        };
        Ok(DataConfirm {
            // TODO: Set a timestamp once we support times TX in the driver.
            timestamp: 0,
            acked,
        })
    }

    /// Extract an MCPS data indication from the given MPDU.
    pub async fn mcps_data_indication(&self, mpdu: MpduFrame) {
        let data_indication = MacIndication::McpsData(DataIndication {
            payload: FramePayload::from_mpdu_frame(mpdu),
            // TODO: Set a timestamp once we support timed RX in the driver.
            timestamp: 0,
        });
        self.indication_sender.send(data_indication).await;
    }
}
