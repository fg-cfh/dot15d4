use parser_info::ParserInfo;

use crate::mpdu::{MpduWithAddressing, MpduWithAllFields, MpduWithSecurity};

mod addressing;
mod frame_control;
mod mpdu;
mod parser_info;

pub use addressing::*;
pub use frame_control::*;
pub use mpdu::*;

/// A marker trait that provides access to addressing fields.
pub trait ParsedUpToAddressing {}
impl ParsedUpToAddressing for MpduWithAddressing {}
impl ParsedUpToAddressing for MpduWithSecurity {}
impl ParsedUpToAddressing for MpduWithAllFields {}

/// A marker trait that provides access to security-related fields.
pub trait ParsedUpToSecurity {}
impl ParsedUpToSecurity for MpduWithSecurity {}
impl ParsedUpToSecurity for MpduWithAllFields {}
