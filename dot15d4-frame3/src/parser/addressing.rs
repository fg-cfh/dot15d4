use core::{fmt::Debug, ops::Range};

use crate::{repr::AddressingRepr, AddressingMode, Error, Result};

/// Short address reader/writer.
///
/// The internal representation is little-endian.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[cfg_attr(feature = "fuzz", derive(arbitrary::Arbitrary))]
pub struct ShortAddress<Bytes>(Bytes);

impl<Bytes: AsRef<[u8]>> ShortAddress<Bytes> {
    pub fn new(bytes: Bytes) -> Self {
        debug_assert_eq!(bytes.as_ref().len(), 2);
        Self(bytes)
    }

    pub fn into_be_bytes(&self) -> [u8; 2] {
        // Safety: Length was checked on instantiation.
        let mut be_bytes = <[u8; 2]>::try_from(self.as_ref()).unwrap();
        be_bytes.reverse();
        be_bytes
    }

    pub fn from_be_bytes(mut be_bytes: [u8; 2]) -> ShortAddress<[u8; 2]> {
        be_bytes.reverse();
        ShortAddress::new(be_bytes)
    }

    pub fn into_u16(&self) -> u16 {
        // Safety: Length was checked on instantiation.
        u16::from_le_bytes(self.0.as_ref().try_into().unwrap())
    }

    pub fn from_u16(short_addr: u16) -> ShortAddress<[u8; 2]> {
        ShortAddress::new(short_addr.to_le_bytes())
    }
}

impl<Bytes: AsRef<[u8]> + AsMut<[u8]>> ShortAddress<Bytes> {
    pub fn set_le_bytes<SrcBytes: AsRef<[u8]>>(&mut self, le_bytes: SrcBytes) {
        debug_assert_eq!(le_bytes.as_ref().len(), 2, "invalid");
        self.as_mut().clone_from_slice(le_bytes.as_ref());
    }

    pub fn set_be_bytes<SrcBytes: AsRef<[u8]>>(&mut self, be_bytes: SrcBytes) {
        debug_assert_eq!(be_bytes.as_ref().len(), 2, "invalid");
        let mut be_bytes = <[u8; 2]>::try_from(be_bytes.as_ref()).expect("invalid");
        be_bytes.reverse();
        self.as_mut().clone_from_slice(be_bytes.as_ref());
    }

    pub fn set_u16(&mut self, short_addr: u16) {
        self.as_mut().clone_from_slice(&short_addr.to_le_bytes());
    }
}

impl<Bytes: AsRef<[u8]>> AsRef<[u8]> for ShortAddress<Bytes> {
    /// Little endian
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl<Bytes: AsMut<[u8]>> AsMut<[u8]> for ShortAddress<Bytes> {
    /// Little endian
    fn as_mut(&mut self) -> &mut [u8] {
        self.0.as_mut()
    }
}

/// Extended address reader/writer.
///
/// The internal representation is little-endian.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[cfg_attr(feature = "fuzz", derive(arbitrary::Arbitrary))]
pub struct ExtendedAddress<Bytes>(Bytes);

impl<Bytes: AsRef<[u8]>> ExtendedAddress<Bytes> {
    pub fn new(bytes: Bytes) -> Self {
        debug_assert_eq!(bytes.as_ref().len(), 8);
        Self(bytes)
    }

    pub fn into_be_bytes(&self) -> [u8; 8] {
        // Safety: Length was checked on instantiation.
        let mut be_bytes = <[u8; 8]>::try_from(self.as_ref()).unwrap();
        be_bytes.reverse();
        be_bytes
    }

    pub fn from_be_bytes(mut be_bytes: [u8; 8]) -> ExtendedAddress<[u8; 8]> {
        be_bytes.reverse();
        ExtendedAddress::new(be_bytes)
    }
}

impl<Bytes: AsRef<[u8]> + AsMut<[u8]>> ExtendedAddress<Bytes> {
    pub fn set_le_bytes<SrcBytes: AsRef<[u8]>>(&mut self, le_bytes: SrcBytes) {
        debug_assert_eq!(le_bytes.as_ref().len(), 8, "invalid");
        self.as_mut().clone_from_slice(le_bytes.as_ref());
    }

    pub fn set_be_bytes<SrcBytes: AsRef<[u8]>>(&mut self, be_bytes: SrcBytes) {
        debug_assert_eq!(be_bytes.as_ref().len(), 8, "invalid");
        let mut be_bytes = <[u8; 2]>::try_from(be_bytes.as_ref()).expect("invalid");
        be_bytes.reverse();
        self.as_mut().clone_from_slice(be_bytes.as_ref());
    }
}

impl<Bytes: AsRef<[u8]>> AsRef<[u8]> for ExtendedAddress<Bytes> {
    /// Little endian
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl<Bytes: AsMut<[u8]>> AsMut<[u8]> for ExtendedAddress<Bytes> {
    /// Little endian
    fn as_mut(&mut self) -> &mut [u8] {
        self.0.as_mut()
    }
}

/// PAN id reader/writer.
///
/// The internal representation is little-endian.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[cfg_attr(feature = "fuzz", derive(arbitrary::Arbitrary))]
pub struct PanId<Bytes>(Bytes);

impl<Bytes: AsRef<[u8]>> PanId<Bytes> {
    pub fn new(bytes: Bytes) -> Self {
        debug_assert_eq!(bytes.as_ref().len(), 2);
        Self(bytes)
    }

    pub fn into_be_bytes(&self) -> [u8; 2] {
        // Safety: Length was checked on instantiation.
        let mut be_bytes = <[u8; 2]>::try_from(self.as_ref()).unwrap();
        be_bytes.reverse();
        be_bytes
    }

    pub fn from_be_bytes(mut be_bytes: [u8; 2]) -> PanId<[u8; 2]> {
        be_bytes.reverse();
        PanId::new(be_bytes)
    }

    pub fn into_u16(&self) -> u16 {
        // Safety: Length was checked on instantiation.
        u16::from_le_bytes(self.0.as_ref().try_into().unwrap())
    }

    pub fn from_u16(short_addr: u16) -> PanId<[u8; 2]> {
        PanId::new(short_addr.to_le_bytes())
    }
}

impl<Bytes: AsRef<[u8]> + AsMut<[u8]>> PanId<Bytes> {
    pub fn set_le_bytes<SrcBytes: AsRef<[u8]>>(&mut self, le_bytes: SrcBytes) {
        debug_assert_eq!(le_bytes.as_ref().len(), 2, "invalid");
        self.as_mut().clone_from_slice(le_bytes.as_ref());
    }

    pub fn set_be_bytes<SrcBytes: AsRef<[u8]>>(&mut self, be_bytes: SrcBytes) {
        debug_assert_eq!(be_bytes.as_ref().len(), 2, "invalid");
        let mut be_bytes = <[u8; 2]>::try_from(be_bytes.as_ref()).expect("invalid");
        be_bytes.reverse();
        self.as_mut().clone_from_slice(be_bytes.as_ref());
    }

    pub fn set_u16(&mut self, short_addr: u16) {
        self.as_mut().clone_from_slice(&short_addr.to_le_bytes());
    }
}

impl<Bytes: AsRef<[u8]>> AsRef<[u8]> for PanId<Bytes> {
    /// Little endian
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl<Bytes: AsMut<[u8]>> AsMut<[u8]> for PanId<Bytes> {
    /// Little endian
    fn as_mut(&mut self) -> &mut [u8] {
        self.0.as_mut()
    }
}

/// An IEEE 802.15.4 address.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[cfg_attr(feature = "fuzz", derive(arbitrary::Arbitrary))]
pub enum Address<Bytes> {
    /// The address is absent.
    Absent,
    /// A short address.
    Short(ShortAddress<Bytes>),
    /// An extended address.
    Extended(ExtendedAddress<Bytes>),
}

impl<Bytes> Address<Bytes> {
    const BROADCAST_ADDR: [u8; 2] = [0xff; 2];

    /// The broadcast address.
    pub fn broadcast_address() -> Address<&'static [u8; 2]> {
        Address::Short(ShortAddress::new(&Self::BROADCAST_ADDR))
    }

    /// Return the length of the address in octets.
    #[allow(clippy::len_without_is_empty)]
    pub fn length(&self) -> usize {
        match self {
            Address::Absent => 0,
            Address::Short(_) => 2,
            Address::Extended(_) => 8,
        }
    }

    /// Query whether the address is absent.
    pub fn is_absent(&self) -> bool {
        matches!(self, Address::Absent)
    }

    /// Query whether the address is short.
    pub fn is_short(&self) -> bool {
        matches!(self, Address::Short(_))
    }

    /// Query whether the address is extended.
    pub fn is_extended(&self) -> bool {
        matches!(self, Address::Extended(_))
    }

    /// Parse an address from a string.
    #[cfg(any(feature = "std", test))]
    pub fn parse(address: &str) -> Result<Self> {
        if address.is_empty() {
            return Ok(Address::Absent);
        }

        let parts: std::vec::Vec<&str> = address.split(':').collect();
        match parts.len() {
            2 => {
                let mut bytes = [0u8; 2];
                for (i, part) in parts.iter().enumerate() {
                    bytes[i] = u8::from_str_radix(part, 16).unwrap();
                }
                Ok(Address::Short(bytes))
            }
            8 => {
                let mut bytes = [0u8; 8];
                for (i, part) in parts.iter().enumerate() {
                    bytes[i] = u8::from_str_radix(part, 16).unwrap();
                }
                Ok(Address::Extended(bytes))
            }
            _ => Err(Error),
        }
    }
}

impl<Bytes: AsRef<[u8]>> Address<Bytes> {
    /// Query whether the address is an unicast address.
    pub fn is_unicast(&self) -> bool {
        !self.is_broadcast()
    }

    /// Query whether this address is the broadcast address.
    pub fn is_broadcast(&self) -> bool {
        match self {
            Address::Absent => false,
            Address::Short(short_address) => {
                *short_address.as_ref() == *Self::BROADCAST_ADDR.as_ref()
            }
            Address::Extended(_) => false,
        }
    }

    /// Derives a short address from an extended address' first two bytes, a
    /// short address remains unchanged.
    ///
    /// TODO: We need to store an explicit extended-to-short address map - at
    ///       least on the coordinator. This approach is not safe.
    ///
    /// Note: This is not an IEEE 802.15.4 standard feature.
    pub fn to_short(&self) -> Option<Address<&[u8]>> {
        match self {
            Address::Short(bytes) => Some(Address::Short(ShortAddress::new(bytes.as_ref()))),
            // Safety: The slice always has the correct size.
            Address::Extended(bytes) => {
                Some(Address::Short(ShortAddress::new(&bytes.as_ref()[..2])))
            }
            _ => None,
        }
    }

    /// Return the address as a slice of bytes.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Address::Absent => &[],
            Address::Short(bytes) => bytes.as_ref(),
            Address::Extended(bytes) => bytes.as_ref(),
        }
    }
}

impl<'bytes, Bytes: AsRef<[u8]> + TryFrom<&'bytes [u8]>> Address<Bytes>
where
    <Bytes as TryFrom<&'bytes [u8]>>::Error: Debug,
{
    /// Create an [`Address`] from a slice of bytes.
    ///
    /// Panics if the given slice is not 0, 2 or 8 bytes long.
    pub fn from_bytes(bytes: &'bytes [u8]) -> Self {
        if bytes.is_empty() {
            Address::Absent
        } else if bytes.len() == 2 {
            // Safety: Slice length has been checked explicitly.
            Address::Short(ShortAddress::new(Bytes::try_from(bytes).unwrap()))
        } else if bytes.len() == 8 {
            // Safety: Slice length has been checked explicitly.
            Address::Extended(ExtendedAddress::new(Bytes::try_from(bytes).unwrap()))
        } else {
            panic!("invalid")
        }
    }
}

impl<Bytes> From<Address<Bytes>> for AddressingMode {
    fn from(value: Address<Bytes>) -> Self {
        match value {
            Address::Absent => AddressingMode::Absent,
            Address::Short(_) => AddressingMode::Short,
            Address::Extended(_) => AddressingMode::Extended,
        }
    }
}

impl<Bytes: AsRef<[u8]>> core::fmt::Display for Address<Bytes> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Address::Absent => write!(f, "absent"),
            Address::Short(bytes) => {
                let bytes = bytes.as_ref();
                write!(f, "{:02x}:{:02x}", bytes[0], bytes[1])
            }
            Address::Extended(bytes) => {
                let bytes = bytes.as_ref();
                write!(
                    f,
                    "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]
                )
            }
        }
    }
}

/// A reader/writer for the IEEE 802.15.4 Addressing Fields.
#[derive(Debug, PartialEq, Eq)]
pub struct AddressingFields<Bytes> {
    dst_pan_id_range: Range<usize>,
    dst_addr_range: Range<usize>,
    src_pan_id_range: Range<usize>,
    src_addr_range: Range<usize>,
    bytes: Bytes,
}

impl<Bytes: AsRef<[u8]>> core::fmt::Display for AddressingFields<Bytes> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "Addressing Fields")?;

        if let Some(dst_pan_id) = self.dst_pan_id() {
            writeln!(f, "  dst pan id: {:0x}", dst_pan_id.into_u16())?;
        }

        if let Some(dst_addr) = self.dst_address() {
            writeln!(f, "  dst address: {}", dst_addr)?;
        }

        if let Some(src_pan_id) = self.src_pan_id() {
            writeln!(f, "  src pan id: {:0x}", src_pan_id.into_u16())?;
        }

        if let Some(src_addr) = self.src_address() {
            writeln!(f, "  src address: {}", src_addr)?;
        }

        Ok(())
    }
}

impl<Bytes: AsRef<[u8]>> AddressingFields<Bytes> {
    /// Create a new [`AddressingFields`] reader/writer from a given buffer.
    ///
    /// # Errors
    ///
    /// This function will check the length of the buffer to ensure it is large
    /// enough to contain the addressing fields. If the buffer is too small, an
    /// error will be returned.
    pub fn new(bytes: Bytes, repr: AddressingRepr) -> Result<Self> {
        let expected_len = repr.addressing_fields_length()? as usize;
        if bytes.as_ref().len() != expected_len {
            return Err(Error);
        }

        // Safety: We checked the length of the given bytes buffer.
        unsafe { Self::new_unchecked(bytes, repr) }
    }

    /// Create a new [`AddressingFields`] reader/writer from a given buffer
    /// without checking the length.
    ///
    /// Safety: Requires the length of the bytes buffer to match the address
    ///         representation exactly.
    pub unsafe fn new_unchecked(bytes: Bytes, repr: AddressingRepr) -> Result<Self> {
        let [dst_pan_id_range, dst_addr_range, src_pan_id_range, src_addr_range] =
            repr.addressing_fields_ranges()?;
        Ok(Self {
            dst_pan_id_range,
            dst_addr_range,
            src_pan_id_range,
            src_addr_range,
            bytes,
        })
    }

    /// Return the length of the Addressing Fields in octets.
    #[allow(clippy::len_without_is_empty)]
    pub fn length(&self) -> usize {
        // Safety: We checked that the length matched exactly when instantiating
        //         the object.
        self.bytes.as_ref().len()
    }

    /// Return the IEEE 802.15.4 destination [`Address`] if not absent.
    pub fn dst_address(&self) -> Option<Address<&[u8]>> {
        self.addr_from_range(self.dst_addr_range.clone())
    }

    /// Return the IEEE 802.15.4 source [`Address`] if not absent.
    pub fn src_address(&self) -> Option<Address<&[u8]>> {
        self.addr_from_range(self.src_addr_range.clone())
    }

    /// Return the IEEE 802.15.4 destination PAN ID if not elided.
    pub fn dst_pan_id(&self) -> Option<PanId<&[u8]>> {
        self.pan_id_from_range(self.dst_pan_id_range.clone())
    }

    /// Return the IEEE 802.15.4 source PAN ID if not elided.
    pub fn src_pan_id(&self) -> Option<PanId<&[u8]>> {
        self.pan_id_from_range(self.src_pan_id_range.clone())
    }

    fn addr_from_range(&self, range: Range<usize>) -> Option<Address<&[u8]>> {
        let addr = &self.bytes.as_ref()[range];
        match addr.len() {
            0 => Some(Address::Absent),
            2 => Some(Address::Short(ShortAddress(addr))),
            4 => Some(Address::Extended(ExtendedAddress(addr))),
            // Safety: This is a guarantee of AddressingRepr.
            _ => unreachable!(),
        }
    }

    fn pan_id_from_range(&self, range: Range<usize>) -> Option<PanId<&[u8]>> {
        let pan_id = &self.bytes.as_ref()[range];
        match pan_id.len() {
            0 => None,
            2 => Some(PanId::new(pan_id)),
            // Safety: This is a guarantee of AddressingRepr.
            _ => unreachable!(),
        }
    }
}

impl<Bytes: AsRef<[u8]> + AsMut<[u8]>> AddressingFields<Bytes> {
    /// Return the IEEE 802.15.4 destination [`Address`] if not absent.
    pub fn dst_address_mut(&mut self) -> Option<Address<&mut [u8]>> {
        self.addr_from_range_mut(self.dst_addr_range.clone())
    }

    /// Return the IEEE 802.15.4 source [`Address`] if not absent.
    pub fn src_address_mut(&mut self) -> Option<Address<&mut [u8]>> {
        self.addr_from_range_mut(self.src_addr_range.clone())
    }

    /// Return the IEEE 802.15.4 destination PAN ID if not elided.
    pub fn dst_pan_id_mut(&mut self) -> Option<PanId<&mut [u8]>> {
        self.pan_id_from_range_mut(self.dst_pan_id_range.clone())
    }

    /// Return the IEEE 802.15.4 source PAN ID if not elided.
    pub fn src_pan_id_mut(&mut self) -> Option<PanId<&mut [u8]>> {
        self.pan_id_from_range_mut(self.src_pan_id_range.clone())
    }

    fn addr_from_range_mut(&mut self, range: Range<usize>) -> Option<Address<&mut [u8]>> {
        let addr = &mut self.bytes.as_mut()[range];
        match addr.len() {
            0 => Some(Address::Absent),
            2 => Some(Address::Short(ShortAddress(addr))),
            4 => Some(Address::Extended(ExtendedAddress(addr))),
            // Safety: This is a guarantee of AddressingRepr.
            _ => unreachable!(),
        }
    }

    fn pan_id_from_range_mut(&mut self, range: Range<usize>) -> Option<PanId<&mut [u8]>> {
        let pan_id = &mut self.bytes.as_mut()[range];
        match pan_id.len() {
            0 => None,
            2 => Some(PanId::new(pan_id)),
            // Safety: This is a guarantee of AddressingRepr.
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_type() {
        assert!(Address::Absent.is_absent());
        assert!(!Address::Absent.is_short());
        assert!(!Address::Absent.is_extended());

        assert!(!Address::Short([0xff, 0xff]).is_absent());
        assert!(Address::Short([0xff, 0xff]).is_short());
        assert!(!Address::Short([0xff, 0xff]).is_extended());

        assert!(!Address::Extended([0xff; 8]).is_absent());
        assert!(!Address::Extended([0xff; 8]).is_short());
        assert!(Address::Extended([0xff; 8]).is_extended());

        assert_eq!(Address::Absent.length(), 0);
        assert_eq!(Address::Short([0xff, 0xff]).length(), 2);
        assert_eq!(Address::Extended([0xff; 8]).length(), 8);
    }

    #[test]
    fn addressing_mode() {
        assert_eq!(AddressingMode::from(0b00), AddressingMode::Absent);
        assert_eq!(AddressingMode::from(0b10), AddressingMode::Short);
        assert_eq!(AddressingMode::from(0b11), AddressingMode::Extended);
        assert_eq!(AddressingMode::from(0b01), AddressingMode::Unknown);

        assert_eq!(AddressingMode::Unknown.size(), 0);
        assert_eq!(AddressingMode::Absent.size(), 0);
        assert_eq!(AddressingMode::Short.size(), 2);
        assert_eq!(AddressingMode::Extended.size(), 8);
    }

    #[test]
    fn is_broadcast() {
        assert!(Address::BROADCAST_ADDR.is_broadcast());
        assert!(Address::Short([0xff, 0xff]).is_broadcast());
        assert!(!Address::Short([0xff, 0xfe]).is_broadcast());

        assert!(!Address::BROADCAST_ADDR.is_unicast());
        assert!(!Address::Short([0xff, 0xff]).is_unicast());
        assert!(Address::Short([0xff, 0xfe]).is_unicast());
    }

    #[test]
    fn as_bytes() {
        assert_eq!(Address::BROADCAST_ADDR.as_bytes(), &[0xff, 0xff]);
        assert_eq!(Address::Short([0xff, 0xff]).as_bytes(), &[0xff, 0xff]);
        assert_eq!(Address::Short([0xff, 0xfe]).as_bytes(), &[0xff, 0xfe]);
        assert_eq!(Address::Extended([0xff; 8]).as_bytes(), &[0xff; 8]);
        assert_eq!(Address::Extended([0x01; 8]).as_bytes(), &[0x01; 8]);
        assert_eq!(Address::Absent.as_bytes(), &[]);
    }

    #[test]
    fn from_bytes() {
        assert_eq!(
            Address::from_bytes(&[0xff, 0xff]),
            Address::Short([0xff, 0xff])
        );
        assert_eq!(
            Address::from_bytes(&[0xff, 0xfe]),
            Address::Short([0xff, 0xfe])
        );
        assert_eq!(
            Address::from_bytes(&[0xff; 8]),
            Address::Extended([0xff; 8])
        );
        assert_eq!(
            Address::from_bytes(&[0x01; 8]),
            Address::Extended([0x01; 8])
        );
        assert_eq!(Address::from_bytes(&[]), Address::Absent);
    }

    #[test]
    #[should_panic]
    fn from_bytes_panic() {
        Address::from_bytes(&[0xff, 0xff, 0xff]);
    }

    #[test]
    fn address_present_flags() {
        use AddressingMode::*;
        use FrameVersion::*;

        macro_rules! check {
            (($version:ident, $dst:ident, $src:ident, $compression:literal) -> $expected:expr) => {
                assert_eq!(
                    AddressingFields::<&[u8], &[u8]>::address_present_flags(
                        $version,
                        $dst,
                        $src,
                        $compression
                    ),
                    $expected
                );
            };
        }

        check!((Ieee802154_2003, Short, Short, false) -> Some((true, Short, true, Short)));
        check!((Ieee802154_2003, Short, Short, true) -> Some((true, Short, false, Short)));
        check!((Ieee802154_2003, Extended, Extended, false) -> Some((true, Extended, true, Extended)));
        check!((Ieee802154_2003, Extended, Extended, true) -> Some((true, Extended, false, Extended)));
        check!((Ieee802154_2003, Short, Extended, false) -> Some((true, Short, true, Extended)));
        check!((Ieee802154_2003, Short, Extended, true) -> Some((true, Short, false, Extended)));
        check!((Ieee802154_2003, Extended, Short, false) -> Some((true, Extended, true, Short)));
        check!((Ieee802154_2003, Extended, Short, true) -> Some((true, Extended, false, Short)));
        check!((Ieee802154_2003, Absent, Short, false) -> Some((false, Absent, true, Short)));
        check!((Ieee802154_2003, Absent, Extended, false) -> Some((false, Absent, true, Extended)));
        check!((Ieee802154_2003, Short, Absent, false) -> Some((true, Short, false, Absent)));
        check!((Ieee802154_2003, Extended, Absent, false) -> Some((true, Extended, false, Absent)));
        check!((Ieee802154_2003, Absent, Short, true) -> None);
        check!((Ieee802154_2003, Absent, Extended, true) -> None);
        check!((Ieee802154_2003, Short, Absent, true) -> None);
        check!((Ieee802154_2003, Extended, Absent, true) -> None);
        check!((Ieee802154_2003, Absent, Absent, false) -> None);
        check!((Ieee802154_2003, Absent, Absent, true) -> None);

        check!((Ieee802154_2006, Short, Short, false) -> Some((true, Short, true, Short)));
        check!((Ieee802154_2006, Short, Short, true) -> Some((true, Short, false, Short)));
        check!((Ieee802154_2006, Extended, Extended, false) -> Some((true, Extended, true, Extended)));
        check!((Ieee802154_2006, Extended, Extended, true) -> Some((true, Extended, false, Extended)));
        check!((Ieee802154_2006, Short, Extended, false) -> Some((true, Short, true, Extended)));
        check!((Ieee802154_2006, Short, Extended, true) -> Some((true, Short, false, Extended)));
        check!((Ieee802154_2006, Extended, Short, false) -> Some((true, Extended, true, Short)));
        check!((Ieee802154_2006, Extended, Short, true) -> Some((true, Extended, false, Short)));
        check!((Ieee802154_2006, Absent, Short, false) -> Some((false, Absent, true, Short)));
        check!((Ieee802154_2006, Absent, Extended, false) -> Some((false, Absent, true, Extended)));
        check!((Ieee802154_2006, Short, Absent, false) -> Some((true, Short, false, Absent)));
        check!((Ieee802154_2006, Extended, Absent, false) -> Some((true, Extended, false, Absent)));
        check!((Ieee802154_2006, Absent, Short, true) -> None);
        check!((Ieee802154_2006, Absent, Extended, true) -> None);
        check!((Ieee802154_2006, Short, Absent, true) -> None);
        check!((Ieee802154_2006, Extended, Absent, true) -> None);
        check!((Ieee802154_2006, Absent, Absent, false) -> None);
        check!((Ieee802154_2006, Absent, Absent, true) -> None);

        check!((Ieee802154_2020, Short, Short, false) -> Some((true, Short, true, Short)));
        check!((Ieee802154_2020, Short, Short, true) -> Some((true, Short, false, Short)));
        check!((Ieee802154_2020, Extended, Extended, false) -> Some((true, Extended, false, Extended)));
        check!((Ieee802154_2020, Extended, Extended, true) -> Some((false, Extended, false, Extended)));
        check!((Ieee802154_2020, Short, Extended, false) -> Some((true, Short, true, Extended)));
        check!((Ieee802154_2020, Short, Extended, true) -> Some((true, Short, false, Extended)));
        check!((Ieee802154_2020, Extended, Short, false) -> Some((true, Extended, true, Short)));
        check!((Ieee802154_2020, Extended, Short, true) -> Some((true, Extended, false, Short)));
        check!((Ieee802154_2020, Absent, Short, false) -> Some((false, Absent, true, Short)));
        check!((Ieee802154_2020, Absent, Extended, false) -> Some((false, Absent, true, Extended)));
        check!((Ieee802154_2020, Short, Absent, false) -> Some((true, Short, false, Absent)));
        check!((Ieee802154_2020, Extended, Absent, false) -> Some((true, Extended, false, Absent)));
        check!((Ieee802154_2020, Absent, Short, true) -> Some((false, Absent, false, Short)));
        check!((Ieee802154_2020, Absent, Extended, true) -> Some((false, Absent, false, Extended)));
        check!((Ieee802154_2020, Short, Absent, true) -> Some((false, Short, false, Absent)));
        check!((Ieee802154_2020, Extended, Absent, true) -> Some((false, Extended, false, Absent)));
        check!((Ieee802154_2020, Absent, Absent, false) -> Some((false, Absent, false, Absent)));
        check!((Ieee802154_2020, Absent, Absent, true) -> Some((true, Absent, false, Absent)));
    }

    #[test]
    fn parse() {
        let mut addresses = vec![
            ("", Address::Absent),
            ("ff:ff", Address::Short([0xff, 0xff])),
            ("ff:fe", Address::Short([0xff, 0xfe])),
            ("ff:ff:ff:ff:ff:ff:ff:ff", Address::Extended([0xff; 8])),
            ("01:01:01:01:01:01:01:01", Address::Extended([0x01; 8])),
            ("00:00:00:00:00:00:00:00", Address::Extended([0x00; 8])),
            (
                "00:00:00:00:00:00:00:01",
                Address::Extended([0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01]),
            ),
        ];

        for (s, expected) in addresses.drain(..) {
            assert_eq!(Address::parse(s).unwrap(), expected);
        }
    }
}
