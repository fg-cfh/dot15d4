//! IEEE 802.15.4 Frame Control related public types.

/// IEEE 802.15.4 frame type.
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
#[cfg_attr(feature = "fuzz", derive(arbitrary::Arbitrary))]
pub enum FrameType {
    /// Beacon frame.
    Beacon = 0b000,
    /// Data frame.
    Data = 0b001,
    /// Acknowledgement frame.
    Ack = 0b010,
    /// MAC command frame.
    MacCommand = 0b011,
    /// Multipurpose frame.
    Multipurpose = 0b101,
    /// Fragmentation frame.
    FragmentOrFrak = 0b110,
    /// Extended frame.
    Extended = 0b111,
    /// Unknown frame type.
    Unknown,
}

impl From<u8> for FrameType {
    fn from(value: u8) -> Self {
        match value {
            0b000 => Self::Beacon,
            0b001 => Self::Data,
            0b010 => Self::Ack,
            0b011 => Self::MacCommand,
            0b101 => Self::Multipurpose,
            0b110 => Self::FragmentOrFrak,
            0b111 => Self::Extended,
            _ => Self::Unknown,
        }
    }
}

/// IEEE 802.15.4 frame version.
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
#[cfg_attr(feature = "fuzz", derive(arbitrary::Arbitrary))]
pub enum FrameVersion {
    /// IEEE 802.15.4-2003 frame version.
    Ieee802154_2003 = 0b00,
    /// IEEE 802.15.4-2006 frame version.
    Ieee802154_2006 = 0b01,
    /// IEEE 802.15.4-2015 and beyond.
    Ieee802154 = 0b10,
    /// Unknown frame version.
    Unknown,
}

impl From<u8> for FrameVersion {
    fn from(value: u8) -> Self {
        match value {
            0b00 => Self::Ieee802154_2003,
            0b01 => Self::Ieee802154_2006,
            0b10 => Self::Ieee802154,
            _ => Self::Unknown,
        }
    }
}
