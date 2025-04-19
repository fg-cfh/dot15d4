#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(feature = "strict", deny(warnings))]
#![allow(dead_code)]

use mpmc_channel::BufferToken;

pub mod addressing;
pub mod driver;
pub mod driver_nrf;
pub mod frame_control;
pub mod mpdu;
pub mod parser;
pub mod payload;
pub mod repr;

pub use addressing::*;
pub use frame_control::*;

/// An error that can occur when reading or writing an IEEE 802.15.4 frame.
#[derive(Debug, Clone, Copy)]
pub struct Error;

/// A type alias for `Result<T, frame::Error>`.
pub type Result<T> = core::result::Result<T, Error>;

/// Generic representation of a buffer-backed entity.
pub trait IntoBuffer {
    /// Consumes the entity and returns the underlying raw buffer.
    fn into_buffer(self) -> BufferToken;
}

/// Generic representation of a buffer-backed structured frame providing access
/// to its protocol data unit (i.e. the frame including its current layer
/// protocol's header and footer).
pub trait FramePdu: IntoBuffer {
    type Pdu: ?Sized;

    /// Exposes a read-only structured representation of the frame's PDU.
    fn pdu_ref(&self) -> &Self::Pdu;

    /// Exposes a mutable structured representation of the frame's PDU.
    fn pdu_mut(&mut self) -> &mut Self::Pdu;
}

/// Generic representation of a buffer-backed structured frame providing access
/// to its service data unit (i.e. its payload) and protocol data unit (i.e. its
/// header and footer).
pub trait Frame: FramePdu {
    /// Retrieves the frame's SDU for reading.
    fn sdu_ref(&self) -> &[u8];

    /// Retrieves the frame's SDU for writing.
    fn sdu_mut(&mut self) -> &mut [u8];
}

#[cfg(test)]
mod test {
    use typenum::Unsigned;

    use super::*;

    #[cfg(feature = "ies")]
    use crate::ie::Ie;
    #[cfg(feature = "security")]
    use crate::sec::{SecurityLevelRepr, SecurityRepr};
    use crate::{
        addressing::{AddressRepr, AddressingRepr, PanIdCompressionRepr},
        driver::{DriverConfig, RadioFrameRepr},
        driver_nrf::{DRIVER_OVERHEAD, FCS_LEN},
        frame_control::{FrameType, FrameVersion},
        mpdu::{imm_ack_frame, MpduRepr, SeqNr, IMM_ACK_BUF_LEN, IMM_ACK_FRAME_REPR},
    };

    #[test]
    fn test_frame_repr_api_and_size() {
        const PAN_ID: u16 = 0x1234;

        let mpdu = MpduRepr::new();

        #[cfg(feature = "security")]
        let mpdu = mpdu.with_security(SecurityRepr::Source4Byte(SecurityLevelRepr::EncMic32));

        #[cfg(not(feature = "security"))]
        let mpdu = mpdu.without_security();

        let mpdu = mpdu.with_frame_config(
            false,
            SeqNr::Yes,
            AddressingRepr::new(
                AddressRepr::Short(PAN_ID),
                AddressRepr::Short(PAN_ID),
                PanIdCompressionRepr::Yes,
            ),
        );

        #[cfg(feature = "ies")]
        let slotframes = [2, 3, 4];

        #[cfg(feature = "ies")]
        let ies = [
            Ie::TimeCorrectionHeaderIe,
            Ie::FullTschTimeslotNestedIe,
            Ie::TschSlotframeAndLinkNestedIe(&slotframes),
        ];

        #[cfg(feature = "ies")]
        let mpdu = mpdu.with_ies(&ies);

        #[cfg(not(feature = "ies"))]
        let mpdu = mpdu.without_ies();

        #[cfg(all(not(feature = "ies"), not(feature = "security")))]
        assert_eq!(size_of_val(&mpdu), 12);

        #[cfg(all(feature = "security", not(feature = "ies")))]
        assert_eq!(size_of_val(&mpdu), 14);

        #[cfg(all(feature = "ies", not(feature = "security")))]
        assert_eq!(size_of_val(&mpdu), 32);

        #[cfg(all(feature = "security", feature = "ies"))]
        assert_eq!(size_of_val(&mpdu), 32);
    }

    const TEST_SEQ_NUM: u8 = 55;

    #[test]
    fn test_imm_ack_frame() {
        const IMM_ACK_LEN: usize = 3;

        let mut buffer = [0; IMM_ACK_BUF_LEN];
        let frame = imm_ack_frame(TEST_SEQ_NUM, &mut buffer);

        assert_eq!(IMM_ACK_BUF_LEN, DRIVER_OVERHEAD + IMM_ACK_LEN + FCS_LEN);
        assert_eq!(
            size_of_val(&frame),
            round_to_alignment(
                size_of_val(&IMM_ACK_FRAME_REPR) + IMM_ACK_BUF_LEN,
                align_of_val(&frame)
            )
        );

        let expected_buffer = vec![
            0,
            FrameType::Ack as u8,
            (FrameVersion::Ieee802154_2006 as u8) << 4,
            TEST_SEQ_NUM,
            0,
            0,
        ];
        assert_eq!(frame.as_bytes(), &expected_buffer);
    }

    fn round_to_alignment(size: usize, alignment: usize) -> usize {
        assert!(alignment > 0 && ((alignment & (alignment - 1)) == 0));

        let size = size as isize;
        let alignment = alignment as isize;

        return ((size + alignment - 1) & -alignment) as usize;
    }
}
