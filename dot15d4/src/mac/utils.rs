use dot15d4_frame3::{
    frame_control::{FrameType, FrameVersion},
    mpdu::MpduFrame,
};

use crate::mac::constants::*;

/// Checks if the current frame is intended for us. For the hardware
/// address, the full 64-bit address should be provided.
pub fn is_frame_for_us(hardware_address: &[u8; 8], mpdu: &MpduFrame) -> bool {
    let fc = mpdu.frame_control();

    // Check if the frame type is valid, otherwise drop.
    if matches!(fc.frame_type(), FrameType::Unknown) {
        return false;
    }

    // Check if the Frame version is valid, otherwise drop.
    if matches!(fc.frame_version(), FrameVersion::Unknown) {
        return false;
    }

    let addresses = mpdu.addressing.addresses(mpdu.frame_control());
    let addr = match addresses.dst_addr {
        AddressRepr::Absent if MAC_IMPLICIT_BROADCAST => AddressRepr::BROADCAST,
        AddressRepr::Short(addr) => AddressRepr::Short(addr),
        AddressRepr::Extended(addr) => AddressRepr::Extended(addr),
        _ => return false,
    };

    // Check if dst_pan (in present) is provided
    let dst_pan_id = addresses.dst_pan_id.unwrap_or(BROADCAST_PAN_ID);
    if dst_pan_id != MAC_PAN_ID && dst_pan_id != BROADCAST_PAN_ID {
        return false;
    }

    // TODO: Check rules if frame comes from PAN coordinator and the same MAC_PAN_ID
    // TODO: Implement `macGroupRxMode` check here
    match addr {
        _ if addr.is_broadcast() => true,
        AddressRepr::Absent => false,
        AddressRepr::Short(addr) => {
            let mut our_short_addr = [0; 2];
            our_short_addr.copy_from_slice(&hardware_address[6..]);
            u16::from_le_bytes(our_short_addr) == addr
        }
        AddressRepr::Extended(addr) => hardware_address == addr,
    }
}
