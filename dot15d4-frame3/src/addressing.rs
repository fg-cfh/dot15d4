//! IEEE 802.15.4 addressing related public types.

/// IEEE 802.15.4 addressing mode.
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
#[cfg_attr(feature = "fuzz", derive(arbitrary::Arbitrary))]
pub enum AddressingMode {
    /// The address is absent.
    Absent = 0b00,
    /// The address is a short address.
    Short = 0b10,
    /// The address is an extended address.
    Extended = 0b11,
    /// Unknown addressing mode.
    Unknown,
}

impl From<u8> for AddressingMode {
    fn from(value: u8) -> Self {
        match value {
            0b00 => Self::Absent,
            0b10 => Self::Short,
            0b11 => Self::Extended,
            _ => Self::Unknown,
        }
    }
}

impl AddressingMode {
    /// Length of an address with the given addressing mode.
    pub const fn length(&self) -> u16 {
        match self {
            AddressingMode::Absent => 0,
            AddressingMode::Short => 2,
            AddressingMode::Extended => 8,
            AddressingMode::Unknown => panic!("unknown"),
        }
    }
}
