use generic_array::ArrayLength;
use mpmc_channel::{BufferAllocator, BufferToken};
use typenum::Unsigned;

use crate::{Frame, FramePdu, IntoBuffer};
use core::{fmt::Debug, marker::PhantomData, mem, num::NonZero, ops::Range};

// PHY
#[allow(dead_code)]
pub const MAX_PHY_PACKET_SIZE_2047: usize = 2048; // SUN, TVWS, RCC, LECIM FSK, and MSK with a 2000 kb/s data rate
#[allow(dead_code)]
pub const MAX_PHY_PACKET_SIZE_127: usize = 127; // all other PHYs

/// Type allowed for [`DriverConfig::Fcs`]
/// Drivers for LECIM, TVWS and SUN PHYs may be configured with a 4-byte FCS, all
pub type FcsFourBytes = u32;

/// Type allowed for [`DriverConfig::Fcs`]
/// Most drivers/PHYs use two bytes.
pub type FcsTwoBytes = u16;

/// Type allowed for [`DriverConfig::Fcs`]
/// Drivers that offload FCS (=CRC) checking to hardware will neither require
/// nor include an FCS in the frame.
pub type FcsNone = ();

#[derive(Clone, Copy, Debug)]
pub struct RadioFrameUnsized;

#[derive(Clone, Copy, Debug)]
pub struct RadioFrameSized;

pub trait DriverConfig {
    /// Any buffer headroom required by the driver.
    type Headroom: ArrayLength;

    /// Any buffer tailroom required by the driver. If the driver takes care of
    /// FCS handling (see [`FcsNone`]), then the tailroom may have to include
    /// the required bytes to let the hardware add the FCS.
    type Tailroom: ArrayLength;

    /// aMaxPhyPacketSize if the FCS is handled by the MAC, otherwise
    /// aMaxPhyPacketSize minus FCS size.
    type MaxFrameLen: ArrayLength;

    /// FCS handling:
    ///  - [`FcsTwoBytes`]: No FCS handling inside the driver or hardware. The
    ///    driver expects the framework to calculate and inject a 2-byte FCS
    ///    into the frame.
    ///  - [`FcsFourBytes`]: No FCS handling inside the driver or hardware. The
    ///    driver expects the framework to calculate and inject a 4-byte FCS
    ///    into the frame.
    ///  - [`FcsNone`]: FCS handling is offloaded to the driver or hardware. The
    ///    driver expects the framework to end the MPDU after the frame payload
    ///    without any FCS. If the driver or hardware requires buffer space for
    ///    its own FCS handling, then it must be included into the tailroom.
    type Fcs: Copy + Debug;
}

/// Provides a simple default radio frame representation implementation.
#[derive(Clone, Copy, Debug)]
pub struct RadioFrameRepr<Config: DriverConfig, State> {
    config: PhantomData<Config>,
    /// Contains [`None`] in state [`RadioFrameUnsized`] and the SDU length in
    /// state [`RadioFrameSized`].
    ///
    /// The SDU length is the driver configuration dependent length of the PSDU
    /// (=MPDU). It contains the FCS length unless FCS calculation is offloaded
    /// to the driver or hardware (see [`FcsNone`])
    ///
    /// Safety: When set, the SDU length must be strictly greater then the
    ///         length of the FCS.
    sdu_length: Option<NonZero<u16>>,
    state: PhantomData<State>,
}

impl<Config: DriverConfig, State> RadioFrameRepr<Config, State> {
    pub const fn headroom_length(&self) -> u16 {
        <Config::Headroom as Unsigned>::U16
    }

    pub const fn tailroom_length(&self) -> u16 {
        <Config::Tailroom as Unsigned>::U16
    }

    pub const fn driver_overhead(&self) -> u16 {
        self.headroom_length() + self.tailroom_length()
    }

    pub const fn max_buffer_length(&self) -> u16 {
        <Config::MaxFrameLen as Unsigned>::U16
    }

    pub const fn fcs_length(&self) -> u16 {
        size_of::<Config::Fcs>() as u16
    }
}

impl<Config: DriverConfig> RadioFrameRepr<Config, RadioFrameUnsized> {
    pub const fn new() -> Self {
        Self {
            config: PhantomData,
            sdu_length: None,
            state: PhantomData,
        }
    }

    pub const fn with_sdu(sdu_length: NonZero<u16>) -> RadioFrameRepr<Config, RadioFrameSized> {
        RadioFrameRepr::<Config, RadioFrameSized>::new(sdu_length)
    }
}

impl<Config: DriverConfig> RadioFrameRepr<Config, RadioFrameSized> {
    pub const fn new(sdu_length: NonZero<u16>) -> Self {
        debug_assert!(sdu_length.get() > size_of::<<Config as DriverConfig>::Fcs>() as u16);
        Self {
            config: PhantomData,
            sdu_length: Some(sdu_length),
            state: PhantomData,
        }
    }

    pub const fn offset_sdu(&self) -> u16 {
        self.headroom_length()
    }

    pub const fn offset_tailroom(&self) -> NonZero<u16> {
        // Safety: The SDU length must be set for a sized radio frame.
        self.sdu_length.unwrap().saturating_add(self.offset_sdu())
    }

    pub const fn pdu_length(&self) -> u16 {
        self.offset_tailroom().get() + self.tailroom_length()
    }

    pub const fn headroom_range(&self) -> Range<u16> {
        0..self.offset_sdu()
    }

    pub const fn tailroom_range(&self) -> Range<usize> {
        self.offset_tailroom().get() as usize..self.pdu_length() as usize
    }

    pub const fn sdu_range(&self) -> Range<usize> {
        self.offset_sdu() as usize..self.offset_tailroom().get() as usize
    }

    pub const fn pdu_range(&self) -> Range<usize> {
        0..self.pdu_length() as usize
    }

    /// Returns the PSDU (=MPDU) length of the frame including the FCS if the
    /// FCS is not offloaded to the driver or hardware, otherwise including the
    /// FCS length.
    ///
    /// This number depends on the driver configuration.
    pub const fn sdu_length(&self) -> NonZero<u16> {
        // Safety: The SDU length must be set for a sized radio frame.
        self.sdu_length.unwrap()
    }

    /// Calculates the PSDU (=MPDU) length of the frame without any FCS.
    ///
    /// This number is independent of the driver configuration.
    pub const fn sdu_wo_fcs_length(&self) -> NonZero<u16> {
        // Safety: We checked on creation that the SDU length is greater than
        //         the FCS length.
        unsafe { NonZero::new_unchecked(self.sdu_length().get() - self.fcs_length()) }
    }
}

/// Provides a simple default radio frame implementation with an externally
/// allocated buffer.
///
/// Note: This frame must not be dropped as it contains a non-droppable buffer.
#[derive(Debug)]
#[must_use = "Must recover the contained buffer after use."]
pub struct RadioFrame<Config: DriverConfig, State> {
    repr: RadioFrameRepr<Config, State>,

    /// The buffer allocated for the frame.
    ///
    /// Safety: For a sized radio frame, the buffer's capacity SHALL be greater
    ///         or equal [`RadioFrameRepr::pdu_length()`], for an unsized
    ///         frame, it must be at least
    ///         [`RadioFrameRepr::max_buffer_length()`].
    buffer: BufferToken,
}

impl<Config: DriverConfig, State> IntoBuffer for RadioFrame<Config, State> {
    fn into_buffer(self) -> BufferToken {
        self.buffer
    }
}

impl<Config: DriverConfig, State> RadioFrame<Config, State> {
    pub fn headroom_length(&self) -> u16 {
        self.repr.headroom_length()
    }
}

impl<Config: DriverConfig> RadioFrame<Config, RadioFrameUnsized> {
    pub fn new(buffer: BufferToken) -> Self {
        let repr = RadioFrameRepr::<Config, RadioFrameUnsized>::new();
        assert!(buffer.len() >= repr.max_buffer_length() as usize);
        Self { repr, buffer }
    }

    pub fn with_size(self, sdu_length: NonZero<u16>) -> RadioFrame<Config, RadioFrameSized> {
        RadioFrame::<Config, RadioFrameSized>::new(self.buffer, sdu_length)
    }
}

impl<Config: DriverConfig> RadioFrame<Config, RadioFrameSized> {
    pub fn new(buffer: BufferToken, sdu_length: NonZero<u16>) -> Self {
        let repr = RadioFrameRepr::<Config, RadioFrameSized>::new(sdu_length);
        assert!(buffer.len() >= repr.pdu_length() as usize);
        Self { repr, buffer }
    }

    pub fn sdu_length(&self) -> NonZero<u16> {
        self.repr.sdu_length()
    }

    pub const fn sdu_wo_fcs_length(&self) -> NonZero<u16> {
        self.repr.sdu_wo_fcs_length()
    }
}

impl<Config: DriverConfig> FramePdu for RadioFrame<Config, RadioFrameSized> {
    type Pdu = [u8];

    fn pdu_ref(&self) -> &[u8] {
        &self.buffer[self.repr.pdu_range()]
    }

    fn pdu_mut(&mut self) -> &mut [u8] {
        &mut self.buffer[self.repr.pdu_range()]
    }
}

impl<Config: DriverConfig> Frame for RadioFrame<Config, RadioFrameSized> {
    fn sdu_ref(&self) -> &[u8] {
        &self.buffer[self.repr.sdu_range()]
    }

    fn sdu_mut(&mut self) -> &mut [u8] {
        &mut self.buffer[self.repr.sdu_range()]
    }
}

/// A droppable version of the radio frame that automatically de-allocates the
/// contained buffer when dropped.
///
/// Note: Use the non-droppable version if possible as it is smaller and allows
///       for zero-allocation buffer re-use. This version mostly exists as it
///       can be conveniently moved into cancellable futures without leaking the
///       buffer.
///
/// TODO: Consider alternatives:
/// - We could implement a generic "DroppableBuffer" and either make the frame
///   generic over the buffer (with a non-droppable buffer as default) or
///   implement the DroppableRadioFrame based on a droppable buffer. This makes
///   sense as soon as we need a droppable buffer elsewhere.
/// - We could implement a "RecoverableRadioFrame" that uses
///   Cell<Option<BufferToken>> internally. It can be passed around by reference
///   which allows the allocating call site to retain ownership while also
///   passing shared ownership (via interior mutability) to the callee. This
///   version would have to implement a method "try_into_buffer()" That recovers
///   the buffer unless the callee had already consumed it.
pub struct DroppableRadioFrame<Config: DriverConfig, State> {
    /// Safety: The option must always be [`Some`] until dropped.
    inner: Option<RadioFrame<Config, State>>,
    allocator: BufferAllocator,
}

impl<Config: DriverConfig, State> Drop for DroppableRadioFrame<Config, State> {
    fn drop(&mut self) {
        // Safety: The radio frame owns the buffer token which can only be
        //         re-surfaced by consuming the frame itself. Therefore, when
        //         the frame is dropped, it has exclusive access to the token.
        unsafe {
            self.allocator
                .deallocate_buffer(self.inner.take().unwrap().into_buffer());
        }
    }
}

impl<Config: DriverConfig, State> IntoBuffer for DroppableRadioFrame<Config, State> {
    fn into_buffer(mut self) -> BufferToken {
        // Safety: The option is always Some until dropped.
        let buffer = self.inner.take().unwrap().into_buffer();

        // We re-use the buffer, so it must no longer be de-allocated on drop.
        mem::forget(self);

        buffer
    }
}

impl<Config: DriverConfig, State> DroppableRadioFrame<Config, State> {
    pub fn headroom_length(&self) -> u16 {
        self.inner.as_ref().unwrap().headroom_length()
    }

    pub fn into_non_droppable_frame(mut self) -> RadioFrame<Config, State> {
        let frame = self.inner.take().unwrap();

        // We re-use the frame, so its buffer must no longer be de-allocated on
        // drop.
        mem::forget(self);

        frame
    }
}

impl<Config: DriverConfig> DroppableRadioFrame<Config, RadioFrameUnsized> {
    pub fn new(buffer: BufferToken, allocator: BufferAllocator) -> Self {
        let inner = Some(RadioFrame::<Config, RadioFrameUnsized>::new(buffer));
        Self { inner, allocator }
    }

    pub fn with_size(
        self,
        sdu_length: NonZero<u16>,
    ) -> DroppableRadioFrame<Config, RadioFrameSized> {
        let allocator = self.allocator;
        let buffer = self.into_buffer();

        let inner = Some(RadioFrame::<Config, RadioFrameSized>::new(
            buffer, sdu_length,
        ));
        DroppableRadioFrame { inner, allocator }
    }
}

impl<Config: DriverConfig> DroppableRadioFrame<Config, RadioFrameSized> {
    pub fn new(buffer: BufferToken, allocator: BufferAllocator, sdu_length: NonZero<u16>) -> Self {
        let inner = Some(RadioFrame::<Config, RadioFrameSized>::new(
            buffer, sdu_length,
        ));
        Self { inner, allocator }
    }

    pub fn sdu_length(&self) -> NonZero<u16> {
        self.inner.as_ref().unwrap().sdu_length()
    }
}

impl<Config: DriverConfig> FramePdu for DroppableRadioFrame<Config, RadioFrameSized> {
    type Pdu = [u8];

    fn pdu_ref(&self) -> &[u8] {
        self.inner.as_ref().unwrap().pdu_ref()
    }

    fn pdu_mut(&mut self) -> &mut [u8] {
        self.inner.as_mut().unwrap().pdu_mut()
    }
}

impl<Config: DriverConfig> Frame for DroppableRadioFrame<Config, RadioFrameSized> {
    fn sdu_ref(&self) -> &[u8] {
        self.inner.as_ref().unwrap().sdu_ref()
    }

    fn sdu_mut(&mut self) -> &mut [u8] {
        self.inner.as_mut().unwrap().sdu_mut()
    }
}
