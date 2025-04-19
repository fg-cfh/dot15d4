use mpmc_channel::BufferToken;

use crate::{
    driver::DriverConfig,
    repr::{mpdu_repr, MpduRepr, SeqNrRepr},
    FrameType, FrameVersion,
};

use super::{MpduFrame, MpduWithIes};

pub const IMM_ACK_FRAME_REPR: MpduRepr<MpduWithIes> = mpdu_repr()
    .with_frame_control(SeqNrRepr::Yes)
    .without_addressing()
    .without_security()
    .without_ies();

/// Allocates an ImmAck frame on the stack and initializes it.
pub fn imm_ack_frame<'ies, Config: DriverConfig>(seq_num: u8, buffer: BufferToken) -> MpduFrame {
    // Safety: We give a valid configuration and therefore expect the operation
    //         not to fail.
    let mut ack_frame = IMM_ACK_FRAME_REPR
        .into_parsed_mpdu::<Config>(FrameVersion::Ieee802154_2006, FrameType::Ack, 0, buffer)
        .unwrap();
    ack_frame.set_sequence_number(seq_num);
    ack_frame.into_mpdu_frame()
}
