use core::{num::NonZero, ops::Range};

use mpmc_channel::BufferToken;
use typenum::Unsigned;

use crate::{
    driver::{DriverConfig, RadioFrame, RadioFrameSized},
    FramePdu, IntoBuffer,
};

/// An unparsed MPDU.
#[must_use = "Must recover the contained buffer after use."]
pub struct MpduFrame {
    pub(crate) buffer: BufferToken,
    /// Contains the buffer offset at which the MPDU starts.
    pub(crate) offset: u16,
    /// Contains the length of the MPDU excluding the FCS.
    pub(crate) length_wo_fcs: NonZero<u16>,
}

impl MpduFrame {
    pub fn new(buffer: BufferToken, offset: u16, length_wo_fcs: NonZero<u16>) -> Self {
        Self {
            buffer,
            offset,
            length_wo_fcs,
        }
    }

    /// Returns the MPDU length of the frame including the FCS if the FCS is not
    /// offloaded to the driver or hardware, otherwise including the FCS length.
    ///
    /// This number depends on the driver configuration.
    pub fn pdu_length<Config: DriverConfig>(&self) -> u16 {
        self.length_wo_fcs.get() + size_of::<<Config as DriverConfig>::Fcs>() as u16
    }

    /// Calculates the MPDU length of the frame without any FCS.
    ///
    /// This number is independent of the driver configuration.
    pub fn pdu_length_wo_fcs(&self) -> u16 {
        self.length_wo_fcs.get()
    }

    fn pdu_range_wo_fcs(&self) -> Range<usize> {
        self.offset as usize..(self.offset + self.pdu_length_wo_fcs()) as usize
    }

    fn pdu_ref_wo_fcs(&self) -> &[u8] {
        &self.buffer[self.pdu_range_wo_fcs()]
    }

    fn pdu_mut_wo_fcs(&mut self) -> &mut [u8] {
        let pdu_range = self.pdu_range_wo_fcs();
        &mut self.buffer[pdu_range]
    }

    /// Produces an unparsed MPDU from a radio frame.
    pub fn from_radio_frame<Config: DriverConfig>(
        radio_frame: RadioFrame<Config, RadioFrameSized>,
    ) -> Self {
        let offset = radio_frame.headroom_length();
        let length_wo_fcs = radio_frame.sdu_wo_fcs_length();
        MpduFrame {
            buffer: radio_frame.into_buffer(),
            offset,
            length_wo_fcs,
        }
    }

    /// Converts an MPDU into a radio frame.
    ///
    /// Calculates the driver-specific FCS if required.
    pub fn into_radio_frame<Config: DriverConfig>(self) -> RadioFrame<Config, RadioFrameSized> {
        // Safety: The length must be set for a sized MPDU.
        debug_assert_eq!(self.offset, <Config::Headroom as Unsigned>::U16);
        // TODO: Calculate the FCS if required.
        RadioFrame::<Config, RadioFrameSized>::new(
            self.buffer,
            self.length_wo_fcs
                .saturating_add(size_of::<<Config as DriverConfig>::Fcs>() as u16),
        )
    }
}

impl IntoBuffer for MpduFrame {
    fn into_buffer(self) -> BufferToken {
        self.buffer
    }
}

impl FramePdu for MpduFrame {
    type Pdu = Self;

    fn pdu_ref(&self) -> &Self {
        self
    }

    fn pdu_mut(&mut self) -> &mut Self {
        self
    }
}
