use core::ops::Range;

use crate::{parser::FrameControl, AddressingMode, Error, Result};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum PanIdCompressionRepr {
    Yes,
    No,
    Legacy,
} // 1 byte

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub struct AddressingRepr {
    pub(crate) dst: AddressingMode,
    pub(crate) src: AddressingMode,
    pub(crate) pan_ids_equal: bool,
    pub(crate) pan_id_compression: PanIdCompressionRepr,
} // 4 bytes

impl AddressingRepr {
    /// Instantiate a new addressing representation.
    ///
    /// Safety: The given addressing modes must be known (i.e. absent, short or
    ///         extended).
    pub const fn new(
        dst: AddressingMode,
        src: AddressingMode,
        pan_ids_equal: bool,
        pan_id_compression: PanIdCompressionRepr,
    ) -> Self {
        if matches!(dst, AddressingMode::Unknown) || matches!(src, AddressingMode::Unknown) {
            panic!("invalid")
        }

        Self {
            dst,
            src,
            pan_ids_equal,
            pan_id_compression,
        }
    }

    /// Instantiate a new addressing representation with pre IEEE 802.15.4-2015
    /// addressing mode.
    ///
    /// Safety: The given addressing modes must be known.
    pub const fn new_legacy_addressing(
        dst: AddressingMode,
        src: AddressingMode,
        pan_ids_equal: bool,
    ) -> Self {
        Self::new(dst, src, pan_ids_equal, PanIdCompressionRepr::Legacy)
    }

    /// Safety: The frame version and addressing modes must be known.
    pub fn from_frame_control<Bytes: AsRef<[u8]>>(
        frame_control: FrameControl<Bytes>,
    ) -> Result<Option<Self>> {
        let dst = frame_control.dst_addressing_mode();
        let src = frame_control.src_addressing_mode();
        let frame_version = frame_control.frame_version();
        let (pan_id_compression, pan_ids_equal) = match frame_version {
            crate::FrameVersion::Ieee802154_2003 | crate::FrameVersion::Ieee802154_2006 => {
                let pan_id_compression = PanIdCompressionRepr::Legacy;

                // See see IEEE 802.15.4-2024, section 7.2.2.6
                let pan_ids_equal = if !matches!(dst, AddressingMode::Absent)
                    && !matches!(src, AddressingMode::Absent)
                {
                    // - If both destination and source addressing information is
                    //   present, the MAC sublayer shall compare the destination and
                    //   source PAN identifiers. If the PAN IDs are identical, the
                    //   PAN ID Compression field shall be set to one, and the
                    //   Source PAN ID field shall be omitted from the transmitted
                    //   frame. If the PAN IDs are different, the PAN ID Compression
                    //   field shall be set to zero, and both Destination PAN ID
                    //   field and Source PAN ID fields shall be included in the
                    //   transmitted frame.
                    frame_control.pan_id_compression()
                } else {
                    // - If only either the destination or the source addressing
                    //   information is present, the PAN ID Compression field shall
                    //   be set to zero, and the PAN ID field of the single address
                    //   shall be included in the transmitted frame.
                    false
                };

                (pan_id_compression, pan_ids_equal)
            }
            crate::FrameVersion::Ieee802154 => {
                let pan_id_compression = if frame_control.pan_id_compression() {
                    PanIdCompressionRepr::Yes
                } else {
                    PanIdCompressionRepr::No
                };

                // If both the destination and source addressing information is present
                // and either is a short address, the MAC sublayer shall compare the
                // destination and source PAN IDs and the PAN ID Compression field shall
                // be set to zero if and only if the PAN identifiers are identical,
                // see IEEE 802.15.4-2024, section 7.2.2.6.
                let pan_ids_equal = if matches!(dst, AddressingMode::Short)
                    || matches!(src, AddressingMode::Short)
                {
                    matches!(pan_id_compression, PanIdCompressionRepr::Yes)
                } else {
                    false
                };

                (pan_id_compression, pan_ids_equal)
            }
            crate::FrameVersion::Unknown => panic!("invalid"),
        };

        let addressing = Self::new(dst, src, pan_ids_equal, pan_id_compression);
        let addressing = if addressing.addressing_fields_length()? == 0 {
            None
        } else {
            Some(addressing)
        };
        Ok(addressing)
    }

    /// Addressing fields length
    pub const fn addressing_fields_length(&self) -> Result<u16> {
        if let Ok([dst_pan_id_len, dst_addr_len, src_pan_id_len, src_addr_len]) =
            self.addressing_fields_lengths()
        {
            // fast const-compat calculation
            Ok(dst_pan_id_len + dst_addr_len + src_pan_id_len + src_addr_len)
        } else {
            Err(Error)
        }
    }

    /// Pan ID compression
    pub const fn pan_id_compression(&self) -> bool {
        match self.pan_id_compression {
            PanIdCompressionRepr::Yes => {
                return true;
            }
            PanIdCompressionRepr::No => {
                return false;
            }
            PanIdCompressionRepr::Legacy => match (self.dst, self.src) {
                (AddressingMode::Short, AddressingMode::Short)
                | (AddressingMode::Short, AddressingMode::Extended)
                | (AddressingMode::Extended, AddressingMode::Short)
                | (AddressingMode::Extended, AddressingMode::Extended) => {
                    if self.pan_ids_equal {
                        true
                    } else {
                        false
                    }
                }

                _ => false,
            },
        }
    }

    /// Destination [`AddressingMode`]
    pub(crate) const fn dst_addr_mode(&self) -> AddressingMode {
        self.dst
    }

    /// Source [`AddressingMode`]
    pub(crate) const fn src_addr_mode(&self) -> AddressingMode {
        self.src
    }

    /// Returns [dst_pan_id_range, dst_address_range, src_pan_id_range, src_address_range]
    pub(crate) const fn addressing_fields_ranges(&self) -> Result<[Range<usize>; 4]> {
        if let Ok([dst_pan_id_len, dst_addr_len, src_pan_id_len, src_addr_len]) =
            self.addressing_fields_lengths()
        {
            let dst_addr_offset = dst_pan_id_len as usize;
            let src_pan_id_offset = dst_addr_offset + dst_addr_len as usize;
            let src_addr_offset = src_pan_id_offset + src_pan_id_len as usize;
            let last_byte = src_addr_offset + src_addr_len as usize;
            Ok([
                0..dst_addr_offset,
                dst_addr_offset..src_pan_id_offset,
                src_pan_id_offset..src_addr_offset,
                src_addr_offset..last_byte,
            ])
        } else {
            Err(Error)
        }
    }

    /// Returns (dst_pan_id_present, dst_address_mode, src_pan_id_present, src_address_mode)
    const fn address_present_flags(&self) -> Result<(bool, AddressingMode, bool, AddressingMode)> {
        use AddressingMode::*;
        match self.pan_id_compression {
            // IEEE 802.15.4-2006 or earlier.
            PanIdCompressionRepr::Legacy => {
                match (self.dst, self.src, self.pan_ids_equal) {
                    // If both destination and source address information is
                    // present, and the destination and source PAN IDs are
                    // identical, then the source PAN ID is omitted.

                    // In the following case, the destination and source PAN IDs
                    // are not identical, and thus both are present.
                    (dst @ (Short | Extended), src @ (Short | Extended), false) => {
                        Ok((true, dst, true, src))
                    }

                    // In the following case, the destination and source PAN IDs
                    // are identical, and thus only the destination PAN ID is
                    // present.
                    (dst @ (Short | Extended), src @ (Short | Extended), true) => {
                        Ok((true, dst, false, src))
                    }

                    // If either the destination or the source address is
                    // present, then the PAN ID of the corresponding address is
                    // present and the PAN ID compression field is set to 0.
                    (Absent, src @ (Short | Extended), _) => Ok((false, Absent, true, src)),
                    (dst @ (Short | Extended), Absent, _) => Ok((true, dst, false, Absent)),

                    // All other cases are invalid.
                    _ => Err(Error),
                }
            }

            // IEEE 802.15.4-2015 and beyond.
            PanIdCompressionRepr::Yes | PanIdCompressionRepr::No => {
                let pan_id_compression =
                    matches!(self.pan_id_compression, PanIdCompressionRepr::Yes);

                match (self.dst, self.src, pan_id_compression) {
                    (Absent, Absent, false) => Ok((false, Absent, false, Absent)),
                    (Absent, Absent, true) => Ok((true, Absent, false, Absent)),
                    (dst, Absent, false) if !matches!(dst, Absent) => {
                        Ok((true, dst, false, Absent))
                    }
                    (dst, Absent, true) if !matches!(dst, Absent) => {
                        Ok((false, dst, false, Absent))
                    }
                    (Absent, src, false) if !matches!(src, Absent) => {
                        Ok((false, Absent, true, src))
                    }
                    (Absent, src, true) if !matches!(src, Absent) => {
                        Ok((false, Absent, false, src))
                    }
                    (Extended, Extended, false) => Ok((true, Extended, false, Extended)),
                    (Extended, Extended, true) => Ok((false, Extended, false, Extended)),
                    (Short, Short, false) => Ok((true, Short, true, Short)),
                    (Short, Extended, false) => Ok((true, Short, true, Extended)),
                    (Extended, Short, false) => Ok((true, Extended, true, Short)),
                    (Short, Extended, true) => Ok((true, Short, false, Extended)),
                    (Extended, Short, true) => Ok((true, Extended, false, Short)),
                    (Short, Short, true) => Ok((true, Short, false, Short)),
                    _ => Err(Error),
                }
            }
        }
    }

    /// Returns [dst_pan_id_len, dst_address_len, src_pan_id_len, src_address_len]
    const fn addressing_fields_lengths(&self) -> Result<[u16; 4]> {
        if let Ok((dst_pan_id_present, dst_address_mode, src_pan_id_present, src_address_mode)) =
            self.address_present_flags()
        {
            Ok([
                if dst_pan_id_present { 2 } else { 0 },
                dst_address_mode.length(),
                if src_pan_id_present { 2 } else { 0 },
                src_address_mode.length(),
            ])
        } else {
            Err(Error)
        }
    }
}
