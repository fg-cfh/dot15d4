use core::{marker::PhantomData, num::NonZero};

#[cfg(feature = "ies")]
use crate::repr::ies::IeListReprVariant;
#[cfg(feature = "security")]
use crate::repr::security::SecurityRepr;
use crate::{
    mpdu::{MpduUnsized, MpduWithAddressing, MpduWithFrameControl, MpduWithIes, MpduWithSecurity},
    Error, Result,
};

use super::{addressing::AddressingRepr, seq_nr::SeqNrRepr};

/// The MPDU representation contains just enough structural information to
/// calculate the required size of an MPDU buffer.
///
/// To read or write content a [`crate::parser::ParsedMpdu`] can be derived via
/// [`MpduRepr::into_parsed_mpdu()`].
///
/// The MPDU representation does not refer to a [`crate::driver::DriverConfig`]
/// so that it can be re-used across drivers.
///
/// The MPDU representation is fully const compatible so that MPDU
/// configurations can be prepared at compile time.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct MpduRepr<'builder, State> {
    pub(crate) seq_nr: SeqNrRepr,
    pub(crate) addressing: Option<AddressingRepr>,
    #[cfg(feature = "security")]
    pub(crate) security: Option<SecurityRepr>,
    #[cfg(feature = "ies")]
    pub(crate) ies: IeListReprVariant<'builder>,
    pub(crate) state: PhantomData<&'builder State>, // Lifetime reference required in case IEs are disabled.
}

impl<'builder> MpduRepr<'builder, MpduUnsized> {
    pub const fn new() -> Self {
        MpduRepr {
            seq_nr: SeqNrRepr::No,
            addressing: None,
            #[cfg(feature = "security")]
            security: None,
            #[cfg(feature = "ies")]
            ies: IeListReprVariant::Empty,
            state: PhantomData,
        }
    }

    pub const fn with_frame_control(
        self,
        seq_nr: SeqNrRepr,
    ) -> MpduRepr<'builder, MpduWithFrameControl> {
        MpduRepr {
            seq_nr,
            addressing: None,
            #[cfg(feature = "security")]
            security: self.security,
            #[cfg(feature = "ies")]
            ies: IeListReprVariant::Empty,
            state: PhantomData,
        }
    }
}

impl<'builder> MpduRepr<'builder, MpduWithFrameControl> {
    pub const fn with_addressing(
        self,
        addressing: AddressingRepr,
    ) -> MpduRepr<'builder, MpduWithAddressing> {
        MpduRepr {
            seq_nr: self.seq_nr,
            addressing: Some(addressing),
            #[cfg(feature = "security")]
            security: self.security,
            #[cfg(feature = "ies")]
            ies: IeListReprVariant::Empty,
            state: PhantomData,
        }
    }

    pub const fn without_addressing(self) -> MpduRepr<'builder, MpduWithAddressing> {
        MpduRepr {
            seq_nr: self.seq_nr,
            addressing: None,
            #[cfg(feature = "security")]
            security: None,
            #[cfg(feature = "ies")]
            ies: self.ies,
            state: PhantomData,
        }
    }
}

impl<'builder> MpduRepr<'builder, MpduWithAddressing> {
    #[cfg(feature = "security")]
    pub const fn with_security(
        self,
        security: SecurityRepr,
    ) -> MpduRepr<'builder, MpduWithSecurity> {
        MpduRepr {
            seq_nr: self.seq_nr,
            addressing: self.addressing,
            security: Some(security),
            #[cfg(feature = "ies")]
            ies: self.ies,
            state: PhantomData,
        }
    }

    pub const fn without_security(self) -> MpduRepr<'builder, MpduWithSecurity> {
        MpduRepr {
            seq_nr: self.seq_nr,
            addressing: self.addressing,
            #[cfg(feature = "security")]
            security: None,
            #[cfg(feature = "ies")]
            ies: self.ies,
            state: PhantomData,
        }
    }
}

impl<'builder> MpduRepr<'builder, MpduWithSecurity> {
    #[cfg(feature = "ies")]
    pub const fn with_ies(
        self,
        ies: IeListReprVariant<'builder>,
    ) -> MpduRepr<'builder, MpduWithIes> {
        MpduRepr {
            seq_nr: self.seq_nr,
            addressing: self.addressing,
            #[cfg(feature = "security")]
            security: self.security,
            ies,
            state: PhantomData,
        }
    }

    pub const fn without_ies(self) -> MpduRepr<'builder, MpduWithIes> {
        MpduRepr {
            seq_nr: self.seq_nr,
            addressing: self.addressing,
            #[cfg(feature = "security")]
            security: self.security,
            #[cfg(feature = "ies")]
            ies: IeListReprVariant::Empty,
            state: PhantomData,
        }
    }
}

impl<'builder> MpduRepr<'builder, MpduWithIes> {
    /// Calculate the MPDU length less the FCS length given the frame payload
    /// length.
    ///
    /// This is convenient when building outgoing frames from scratch.
    ///
    /// Validates addressing for consistency.
    ///
    /// If the representation's IE list contains IEs with termination then those
    /// IEs will be validated on the fly.
    pub const fn mpdu_length_wo_fcs(&self, frame_payload_length: u16) -> Result<NonZero<u16>> {
        let mut len = match self.mpdu_less_ies_and_payload_length() {
            Ok(len) => len,
            Err(e) => {
                return Err(e);
            }
        };

        #[cfg(feature = "ies")]
        {
            len += match self.ies.ies_length(frame_payload_length > 0) {
                Ok(len) => len,
                Err(e) => {
                    return Err(e);
                }
            }
        }

        len += frame_payload_length;

        // Safety: The above calculation will always yield a non-wrapped,
        //         non-zero u16.
        Ok(unsafe { NonZero::new_unchecked(len) })
    }

    /// Calculate the IEs and frame payload length given the MPDU length.
    ///
    /// This is convenient when parsing incoming frames.
    ///
    /// Validates addressing for consistency.
    ///
    /// If the representation's IE list contains IEs without termination then
    /// the calculation becomes non-deterministic due to optional payload
    /// termination and will fail. Also fails if the given IEs are invalid or
    /// inconsistent.
    pub const fn ies_and_frame_payload_length(
        &self,
        mpdu_length_wo_fcs: u16,
    ) -> Result<(u16, u16)> {
        let mpdu_less_ies_and_payload_length = match self.mpdu_less_ies_and_payload_length() {
            Ok(len) => len,
            Err(e) => {
                return Err(e);
            }
        };

        if mpdu_less_ies_and_payload_length > mpdu_length_wo_fcs {
            return Err(Error);
        }

        #[cfg(feature = "ies")]
        {
            let mpdu_ies_and_payload_length = mpdu_length_wo_fcs - mpdu_less_ies_and_payload_length;
            self.ies
                .ies_and_frame_payload_length(mpdu_ies_and_payload_length)
        }

        #[cfg(not(feature = "ies"))]
        Ok((0, mpdu_length_wo_fcs - mpdu_less_ies_and_payload_length))
    }

    /// Internal helper that calculates the MPDU length less IEs, frame payload
    /// and FCS, i.e. including frame control, sequence number, addressing, aux
    /// sec header and MIC.
    ///
    /// Validates addressing for consistency.
    ///
    /// This is required both, when parsing incoming frames as well as when
    /// building outgoing frames.
    const fn mpdu_less_ies_and_payload_length(&self) -> Result<u16> {
        const FRAME_CONTROL_LEN: u16 = 2;

        let mut len = FRAME_CONTROL_LEN + self.seq_nr.length();

        len += match &self.addressing {
            Some(addressing) => match addressing.addressing_fields_length() {
                Ok(addressing_fields_length) => addressing_fields_length,
                e => {
                    return e;
                }
            },
            None => 0,
        };

        #[cfg(feature = "security")]
        {
            len += match &self.security {
                Some(security) => security.aux_sec_header_length() + security.mic_length(),
                None => 0,
            };
        }

        // Safety: The above calculation will always yield a non-wrapped,
        //         non-zero u16.
        Ok(len)
    }
}

pub const fn mpdu_repr<'builder>() -> MpduRepr<'builder, MpduUnsized> {
    MpduRepr::new()
}
