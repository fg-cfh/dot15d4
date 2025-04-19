use core::{num::NonZero, ops::Range};

use mpmc_channel::BufferToken;

use crate::{mpdu::MpduFrame, FramePdu, IntoBuffer};

/// A zerocopy frame with enough buffer capacity for a full radio driver frame
/// but only exposing the slice available for frame payload.
#[must_use = "Must recover the contained buffer after use."]
pub struct FramePayload {
    buffer: BufferToken,
    /// Contains the buffer offset at which the frame payload starts.
    offset: u16,
    /// Contains the length of the frame payload.
    length: NonZero<u16>,
}

/// The frame control and sequence number fields are available on any MPDU.
impl FramePayload {
    pub fn new(buffer: BufferToken, offset: u16, length: NonZero<u16>) -> Self {
        Self {
            buffer,
            offset,
            length,
        }
    }

    fn pdu_length(&self) -> u16 {
        // Safety: A length must be present for a sized MPDU.
        self.length.get()
    }

    fn pdu_range(&self) -> Range<usize> {
        self.offset as usize..(self.offset + self.pdu_length()) as usize
    }

    fn pdu_ref(&self) -> &[u8] {
        &self.buffer[self.pdu_range()]
    }

    fn pdu_mut(&mut self) -> &mut [u8] {
        let pdu_range = self.pdu_range();
        &mut self.buffer[pdu_range]
    }

    /// Produces an MCPS frame from a parsed MPDU data frame.
    // TODO: Requires a parsed MPDU abstraction.
    pub fn from_mpdu_frame(_mpdu: MpduFrame) -> Self {
        // TODO: implement
        todo!()
    }

    /// Converts an MCPS frame into a parsed MPDU data frame.
    // TODO: Requires a parsed MPDU abstraction.
    pub fn into_mpdu_frame(self) -> MpduFrame {
        // TODO: implement
        todo!()
    }
}

impl IntoBuffer for FramePayload {
    fn into_buffer(self) -> BufferToken {
        self.buffer
    }
}

impl FramePdu for FramePayload {
    type Pdu = Self;

    fn pdu_ref(&self) -> &Self {
        self
    }

    fn pdu_mut(&mut self) -> &mut Self {
        self
    }
}
