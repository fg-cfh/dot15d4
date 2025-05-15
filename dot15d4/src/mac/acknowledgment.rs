use core::num::NonZeroU16;

use crate::{
    mac::{
        constants::{MAC_AIFS_PERIOD, MAC_SIFS_PERIOD},
        MacService,
    },
    radio::MAX_DRIVER_OVERHEAD,
    sync::{select, Either},
    time::Duration,
};
use dot15d4_frame3::{
    driver::{DriverConfig, RadioFrame, RadioFrameRepr, RadioFrameSized, RadioFrameUnsized},
    frame_control::FrameType,
    mpdu::{imm_ack_frame, MpduFrame, IMM_ACK_FRAME_REPR},
};
use embedded_hal_async::delay::DelayNs;
use rand_core::RngCore;

use super::MacBufferAllocator;

pub(crate) const ACK_MPDU_LENGTH: NonZeroU16 = IMM_ACK_FRAME_REPR.mpdu_length(0, false);
pub(crate) const ACK_MPDU_SIZE: usize = ACK_MPDU_LENGTH.get() as usize;
pub(crate) const MAX_ACK_BUFFER_SIZE: usize = ACK_MPDU_SIZE + MAX_DRIVER_OVERHEAD;

impl<'svc, Rng: RngCore, TIMER: DelayNs + Clone, Config: DriverConfig>
    MacService<'svc, Rng, TIMER, Config>
{
    /// Calculates the exact buffer size required for an ACK frame with the
    /// given driver configuration.
    const fn ack_buffer_size() -> usize {
        ACK_MPDU_SIZE
            + RadioFrameRepr::<Config, RadioFrameUnsized>::new().driver_overhead() as usize
    }

    pub(crate) fn allocate_tx_ack(
        buffer_allocator: MacBufferAllocator,
    ) -> RadioFrame<Config, RadioFrameSized> {
        imm_ack_frame::<Config>(
            0,
            buffer_allocator
                .try_allocate_buffer(Self::ack_buffer_size())
                .expect("no capacity"),
        )
        .into_radio_frame()
    }

    /// Transmit acknowledgment for a frame that has been received if that frame
    /// requests acknowledgement and contains a valid sequence number.
    ///
    /// Blocks until the acknowledgment has been transmitted over the radio.
    ///
    /// * `rx_mpdu` - Frame to be acknowledged
    pub(crate) async fn transmit_ack(&self, rx_mpdu: &MpduFrame) {
        if !rx_mpdu.frame_control().ack_request() {
            return;
        }

        let seq_num = rx_mpdu.sequence_number();
        if seq_num.is_none() {
            // TODO: This is an invalid frame which should be logged.
            return;
        }

        // Safety: This function uses the pre-allocated Tx ACK frame
        //         exclusively.
        let mut tx_ack_mpdu = MpduFrame::from_radio_frame(self.tx_ack_frame.take().unwrap());
        tx_ack_mpdu.set_sequence_number(seq_num.unwrap());

        let tx_ack_frame = tx_ack_mpdu.into_radio_frame();
        let reusable_tx_ack_frame = self.radio_send(tx_ack_frame).await;

        self.tx_ack_frame.set(Some(reusable_tx_ack_frame));
    }

    /// Wait for the reception of an acknowledgment for a specific sequence
    /// number. Time out if ack is not received within a specific delay.
    /// Return `true` if such an ack is received, return `else` otherwise (or
    /// if timed out).
    ///
    /// * `sequence_number` - Sequence number of the frame waiting for ack
    pub(crate) async fn wait_for_ack(&self, sequence_number: u8) -> bool {
        let mut timer = self.timer.clone();

        // We expect an ACK to come back AIFS + time for an ACK to travel + SIFS (guard)
        //
        // An Imm-ACK is 3 bytes + 6 bytes (PHY header) long and therefore
        // should take around 288us at 250kbps to get back (2.4G O-QPSK).
        //
        // TODO: Calculate the delay based on PHY-specific AIFS, SIFS and symbol
        //       period parameters provided by the driver.
        let delay = MAC_AIFS_PERIOD + MAC_SIFS_PERIOD + Duration::from_us(288);

        match select::select(
            async {
                // We may receive multiple frames during that period of time.
                // non-matching frames are dropped.
                // TODO: Non-matching frames should be handled normally, as the
                //       ACK could simply have been lost and we're now dropping
                //       legit frames from other devices until the timeout fires.
                //       Actually, if we receive a non-ACK frame there's no use
                //       in waiting for another ACK as ACKs are not resent. If
                //       we change this, then we need to pass in a full-sized
                //       buffer of course.
                loop {
                    let rx_ack_mpdu =
                        MpduFrame::from_radio_frame(self.radio_recv(Self::ack_buffer_size()).await);

                    if !matches!(rx_ack_mpdu.frame_control().frame_type(), FrameType::Ack) {
                        // TODO: Don't drop the received frame but handle it,
                        //       see above.
                        continue;
                    }

                    let seq_num = rx_ack_mpdu.sequence_number();
                    if seq_num.is_none() {
                        // TODO: log invalid frame
                        continue;
                    }

                    if sequence_number == seq_num.unwrap() {
                        // TODO: Don't continue but report NACK.
                        break;
                    }
                }
            },
            // Timeout for waiting on an ACK
            async {
                timer.delay_us(delay.as_us() as u32).await;
                info!("Expired !");
            },
        )
        .await
        {
            Either::First(_) => true,
            Either::Second(_) => false,
        }
    }

    /// Check if the given frame needs to be acknowledged, based on current
    /// buffer content and frame addressing. If so, acknowledgment request is
    /// set in the frame.
    ///
    /// * `frame` - MPDU to check and update, if necessary.
    pub(crate) fn set_ack(mpdu: &mut MpduFrame) -> Option<u8> {
        match mpdu.addressing().and_then(|addr| addr.dst_address()) {
            Some(addr) if addr.is_unicast() => {
                mpdu.frame_control().set_ack_request(true);
                mpdu.sequence_number()
            }
            _ => None,
        }
    }
}
