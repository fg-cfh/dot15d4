use mpmc_channel::BufferToken;

#[cfg(feature = "ies")]
use crate::repr::IeListReprVariant;
use crate::{
    driver::{DriverConfig, RadioFrameRepr, RadioFrameSized},
    frame_control::{FrameType, FrameVersion},
    mpdu::{
        MpduFrame, MpduWithAddressing, MpduWithAllFields, MpduWithFrameControl, MpduWithIes,
        MpduWithSecurity,
    },
    repr::{AddressingRepr, MpduRepr, SeqNrRepr},
    Frame, FramePdu, IntoBuffer, Result,
};

use super::{AddressingFields, FrameControl, ParsedUpToAddressing, ParsedUpToSecurity, ParserInfo};

/// Accessors into fields that are available on an unparsed MPDU frame.
impl MpduFrame {
    fn frame_control_slice_ref(&self) -> &[u8] {
        let parser_info = ParserInfo::new(self.offset, SeqNrRepr::No);
        &self.buffer[parser_info.range_frame_control()]
    }

    pub(crate) fn frame_control_slice_mut(&mut self) -> &mut [u8] {
        let parser_info = ParserInfo::new(self.offset, SeqNrRepr::No);
        &mut self.buffer[parser_info.range_frame_control()]
    }

    /// Provides read-only access to the [`FrameControl`] field.
    pub fn frame_control(&self) -> FrameControl<&[u8]> {
        FrameControl::new_unchecked(self.frame_control_slice_ref())
    }

    /// Provides mutable access to the [`FrameControl`] field.
    pub fn frame_control_mut(&mut self) -> FrameControl<&mut [u8]> {
        FrameControl::new_unchecked(self.frame_control_slice_mut())
    }

    /// Reads the sequence number field.
    pub fn sequence_number(&self) -> Option<u8> {
        if self.frame_control().sequence_number_suppression() {
            return None;
        }

        // Safety: We now know for sure that a sequence number is present.
        let parser_info = ParserInfo::new(self.offset, SeqNrRepr::Yes);
        Some(self.buffer[parser_info.offset_seq_nr().get() as usize])
    }

    /// Writes the sequence number field.
    ///
    /// Safety: Requires the sequence number suppression field of the frame
    ///         control field to be false.
    pub fn set_sequence_number(&mut self, seq_num: u8) {
        debug_assert!(!self.frame_control().sequence_number_suppression());

        let parser_info = ParserInfo::new(self.offset, SeqNrRepr::Yes);
        self.buffer[parser_info.offset_seq_nr().get() as usize] = seq_num;
    }
}

/// A partially or fully parsed MPDU that provides staged access to frame
/// content.
pub struct ParsedMpdu<State> {
    parser_info: ParserInfo<State>,
    mpdu: MpduFrame,
}

impl<'repr> MpduRepr<'repr, MpduWithIes> {
    /// Allocates and initializes an MPDU reader/writer based on this MPDU
    /// representation.
    ///
    /// This is used to build an MPDU from scratch.
    ///
    /// All structural information that can be derived from the MPDU
    /// representation or function arguments will be written into the buffer:
    /// - All sub-fields of the frame control field will be initialized with
    ///   known values except "Frame Pending" and "AR" which will be initialized
    ///   to zero.
    /// - The security control field of the auxiliary security header will be
    ///   initialized.
    /// - All header, payload and nested IEs will be pre-initialized with their
    ///   length, id and type.
    /// - Information required to determine the structure and size of
    ///   dynamically sized IEs will be pre-initialized, namely the number of
    ///   channels and the extended bitmap length for the long version of a
    ///   Channel Hopping IE and the number of slotframes and links within each
    ///   slotframe for a TSCH Slotframe and Link IE.
    pub fn into_parsed_mpdu<Config: DriverConfig>(
        &self,
        frame_version: FrameVersion,
        frame_type: FrameType,
        frame_payload_length: u16,
        // Note: The buffer may or may not be zeroed.
        buffer: BufferToken,
    ) -> Result<ParsedMpdu<MpduWithAllFields>> {
        let mpdu_length_wo_fcs = self.mpdu_length_wo_fcs(frame_payload_length)?;

        let radio_frame_repr = RadioFrameRepr::<Config, RadioFrameSized>::new(mpdu_length_wo_fcs);
        let min_buffer_length = radio_frame_repr.pdu_length();
        assert!(buffer.len() >= min_buffer_length as usize);

        let offset_mpdu = radio_frame_repr.headroom_length();
        let mut mpdu = MpduFrame::new(buffer, offset_mpdu, mpdu_length_wo_fcs);

        let seq_num_suppression = matches!(self.seq_nr, SeqNrRepr::No);
        let (dst_addr_mode, src_addr_mode, pan_id_compression) = match &self.addressing {
            Some(addressing) => (
                addressing.dst_addr_mode(),
                addressing.src_addr_mode(),
                addressing.pan_id_compression(),
            ),
            // Safety: Addressing must be configured for a builder in MpduSized
            //         state.
            None => unreachable!(),
        };

        #[cfg(feature = "security")]
        let security_enabled = self.security.is_some();
        #[cfg(not(feature = "security"))]
        let security_enabled = false;

        #[cfg(feature = "ies")]
        let ie_present = !self.ies.is_empty();
        #[cfg(not(feature = "ies"))]
        let ie_present = false;

        // All frame control fields shall default to zero.
        mpdu.frame_control_slice_mut().fill(0);

        let mut fc = mpdu.frame_control_mut();
        fc.set_frame_version(frame_version);
        fc.set_frame_type(frame_type);
        fc.set_sequence_number_suppression(seq_num_suppression);
        fc.set_pan_id_compression(pan_id_compression);
        fc.set_dst_addressing_mode(dst_addr_mode);
        fc.set_src_addressing_mode(src_addr_mode);
        fc.set_security_enabled(security_enabled);
        fc.set_information_elements_present(ie_present);

        // TODO: Initialize other fields mentioned in the method docs.

        let parser_info = ParserInfo::new(offset_mpdu, self.seq_nr);

        let parser_info = if let Some(addressing) = self.addressing {
            parser_info.with_addressing(addressing)?
        } else {
            parser_info.without_addressing()
        };

        #[cfg(feature = "security")]
        let parser_info = if let Some(security) = self.security {
            parser_info.with_security(security)
        } else {
            parser_info.without_security()
        };
        #[cfg(not(feature = "security"))]
        let parser_info = parser_info.without_security();

        #[cfg(feature = "ies")]
        let parser_info = match self.ies {
            IeListReprVariant::Empty => {
                parser_info.without_ies_with_payload_length::<Config>(frame_payload_length)
            }
            ies => parser_info.with_ies_and_payload_length::<Config>(ies, frame_payload_length)?,
        };
        #[cfg(not(feature = "ies"))]
        let parser_info = parser_info.without_ies::<Config>(frame_payload_length);

        Ok(ParsedMpdu { parser_info, mpdu })
    }
}

impl ParsedMpdu<MpduWithFrameControl> {
    /// Initializes a partially parsed MPDU with access to the frame control and
    /// sequence number fields from an unparsed MPDU.
    ///
    /// This is used to parse an incoming MPDU.
    pub fn new(mpdu: MpduFrame) -> Self {
        let frame_control = mpdu.frame_control();
        let seq_nr = if frame_control.sequence_number_suppression() {
            SeqNrRepr::Yes
        } else {
            SeqNrRepr::No
        };
        Self {
            parser_info: ParserInfo::new(mpdu.offset, seq_nr),
            mpdu,
        }
    }

    /// Parses the frame control field to identify the addressing configuration
    /// of the MPDU.
    pub fn parse_addressing(self) -> Result<ParsedMpdu<MpduWithAddressing>> {
        let addressing = AddressingRepr::from_frame_control(self.frame_control())?;
        let parser_info = if let Some(addressing) = addressing {
            self.parser_info.with_addressing(addressing)?
        } else {
            self.parser_info.without_addressing()
        };

        Ok(ParsedMpdu {
            parser_info,
            mpdu: self.mpdu,
        })
    }
}

impl ParsedMpdu<MpduWithAddressing> {
    /// Parses the frame control field to identify the security configuration of
    /// the MPDU.
    pub fn parse_security(self) -> ParsedMpdu<MpduWithSecurity> {
        // TODO: implement
        debug_assert!(!self.frame_control().security_enabled());

        ParsedMpdu {
            parser_info: self.parser_info.without_security(),
            mpdu: self.mpdu,
        }
    }
}

impl ParsedMpdu<MpduWithSecurity> {
    /// Parses the frame control and information element fields to identify the
    /// information elements of the MPDU.
    pub fn parse_ies<Config: DriverConfig>(self) -> Result<ParsedMpdu<MpduWithAllFields>> {
        // TODO: implement
        debug_assert!(!self.frame_control().information_elements_present());

        let mpdu_length_wo_fcs = self.mpdu.pdu_length_wo_fcs();

        Ok(ParsedMpdu {
            parser_info: self
                .parser_info
                .without_ies_with_mpdu_length::<Config>(mpdu_length_wo_fcs)?,
            mpdu: self.mpdu,
        })
    }
}

/// A proxy to fields accessible from an MPDU in any parsing state.
///
/// Note: Only parts of the frame control field that don't change the MPDU
///       structurally may be edited. All other fields must be initialized via
///       [`MpduRepr::into_parsed_mpdu()`].
impl<State> ParsedMpdu<State> {
    /// See [`MpduFrame::frame_control()`].
    pub fn frame_control(&self) -> FrameControl<&[u8]> {
        self.mpdu.frame_control()
    }

    /// See [`FrameControl::set_ack_request()`]
    pub fn set_ack_request(&mut self, ack_request: bool) {
        self.mpdu.frame_control_mut().set_ack_request(ack_request);
    }

    /// See [`FrameControl::set_frame_pending()`]
    pub fn set_frame_pending(&mut self, frame_pending: bool) {
        self.mpdu
            .frame_control_mut()
            .set_frame_pending(frame_pending);
    }

    /// See [`MpduFrame::sequence_number()`].
    pub fn sequence_number(&self) -> Option<u8> {
        self.mpdu.sequence_number()
    }

    /// See [`MpduFrame::set_sequence_number()`].
    pub fn set_sequence_number(&mut self, seq_num: u8) {
        // Safety: We synchronize the frame representation and the frame
        //         control field when we instantiate the parsed MPDU. The
        //         following call checks the frame control field's sequence
        //         number suppression field.
        self.mpdu.set_sequence_number(seq_num)
    }
}

/// Exposes fields accessible from an MPDU once the addressing configuration is
/// known.
impl<State: ParsedUpToAddressing> ParsedMpdu<State> {
    /// Addressing field access.
    pub fn addressing_fields(&self) -> Result<Option<AddressingFields<&[u8]>>> {
        let addressing_repr = match AddressingRepr::from_frame_control(self.frame_control())? {
            Some(addressing_repr) => addressing_repr,
            None => {
                return Ok(None);
            }
        };

        let addressing_fields = match self.parser_info.range_addressing() {
            Some(range_addressing) => {
                // Safety: Both, the addressing representation and the addressing
                //         range are synced with the frame control field.
                unsafe {
                    AddressingFields::<&[u8]>::new_unchecked(
                        &self.mpdu.buffer[range_addressing],
                        addressing_repr,
                    )?
                }
            }
            // Safety: The frame control field and the frame representation
            //         should be synced.
            None => unreachable!(),
        };

        Ok(Some(addressing_fields))
    }
}

/// Exposes fields accessible from an MPDU once the security configuration is
/// known.
impl<State: ParsedUpToSecurity> ParsedMpdu<State> {
    // TODO: Add access to the aux security header and MIC.
}

impl ParsedMpdu<MpduWithAllFields> {
    // TODO: Add access to IEs.

    pub fn frame_payload(&self) -> Option<&[u8]> {
        Some(&self.mpdu.buffer[self.parser_info.range_frame_payload()?])
    }

    pub fn frame_payload_mut(&mut self) -> Option<&mut [u8]> {
        Some(&mut self.mpdu.buffer[self.parser_info.range_frame_payload()?])
    }

    pub fn fcs(&self) -> Option<&[u8]> {
        Some(&self.mpdu.buffer[self.parser_info.range_fcs()?])
    }

    pub fn fcs_mut(&mut self) -> Option<&mut [u8]> {
        Some(&mut self.mpdu.buffer[self.parser_info.range_fcs()?])
    }

    pub fn into_mpdu_frame(self) -> MpduFrame {
        self.mpdu
    }
}

impl<State> IntoBuffer for ParsedMpdu<State> {
    fn into_buffer(self) -> BufferToken {
        self.mpdu.into_buffer()
    }
}

impl<State> FramePdu for ParsedMpdu<State> {
    type Pdu = Self;

    fn pdu_ref(&self) -> &Self {
        self
    }

    fn pdu_mut(&mut self) -> &mut Self {
        self
    }
}

impl Frame for ParsedMpdu<MpduWithAllFields> {
    /// Provides access to the frame payload for reading.
    fn sdu_ref(&self) -> &[u8] {
        self.frame_payload().unwrap_or(&[])
    }

    /// Provides access to the frame payload for writing.
    fn sdu_mut(&mut self) -> &mut [u8] {
        self.frame_payload_mut().unwrap_or(&mut [])
    }
}
