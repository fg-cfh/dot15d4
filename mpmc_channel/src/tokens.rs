use core::{
    mem,
    ops::{Deref, DerefMut},
};

/// A non-cloneable token representing a message slot allocated for sending a
/// request: Produced by allocating a slot and consumed by sending a request
/// across the channel.  Guarantees bandwidth on the channel for a single
/// request.
#[must_use = "Must be returned to the channel to send a request."]
pub struct RequestToken(u8, TokenGuard);

impl RequestToken {
    /// Creates a new request token.
    pub(crate) fn new(msg_slot: u8) -> Self {
        Self(msg_slot, TokenGuard)
    }

    /// Consume the token.
    pub(crate) fn consume(self) -> u8 {
        self.1.consume();
        self.0
    }

    /// Returns an opaque number representing the message slot allocated for the
    /// request.
    ///
    /// This can be used to manage resources based on message slot allocation.
    pub fn message_slot(&self) -> u8 {
        self.0
    }
}

/// A non-cloneable token representing a message slot allocated for receiving a
/// request and returning a response: Produced by receiving a request and consumed
/// by returning a response across the channel. Guarantees bandwidth on the
/// channel for a single response.
#[must_use = "Must be returned to the channel to send a response."]
pub struct ResponseToken(u8, TokenGuard);

impl ResponseToken {
    /// Creates a new response token.
    pub(crate) fn new(msg_slot: u8) -> Self {
        Self(msg_slot, TokenGuard)
    }

    /// Consume the token.
    pub(crate) fn consume(self) -> u8 {
        self.1.consume();
        self.0
    }

    /// Returns an opaque number representing the message slot allocated for the
    /// response.
    ///
    /// This can be used to manage resources based on message slot allocation.
    pub fn message_slot(&self) -> u8 {
        self.0
    }
}

/// A non-cloneable token representing an allocated consumer slot: Produced by
/// allocating a consumer slot. Must be presented to receive a request from the
/// channel. Guarantees bandwidth on the channel for a single consumer.
#[must_use = "Must be presented to the channel to access the consumer slot."]
pub struct ConsumerToken(u8, TokenGuard);

impl ConsumerToken {
    /// Creates a new consumer token.
    pub(crate) fn new(msg_slot: u8) -> Self {
        Self(msg_slot, TokenGuard)
    }

    /// Consume the token.
    pub(crate) fn consume(self) -> u8 {
        self.1.consume();
        self.0
    }

    /// Returns the consumer slot represented by this token.
    pub(crate) fn consumer_slot(&self) -> u8 {
        self.0
    }
}

/// A token representing a non-cloneable, zerocopy, 1-aligned byte buffer that
/// safely fakes static lifetime so that it can be passed around without
/// polluting channels' and other framework structures' lifetimes.
///
/// This buffer is intended to behave as a smart pointer wrapping a previously
/// allocated &'static mut [u8].
///
/// Other than in the case of [`allocator_api2::boxed::Box`] the contained
/// buffer is not automatically de-allocated when the token is dropped. The
/// token must be manually returned to the allocator from which it was
/// allocated. This allows us to keep channels' and other framework structures'
/// generics free from the allocator type and avoid the runtime cost of carrying
/// a reference to the allocator in our messages.
///
/// Safety:
///   - The token must not be dropped but needs to be returned manually to the
///     allocator from which it was allocated once it is no longer being used.
///   - The token does not implement into_inner() but only Deref/DerefMut.
///     Thereby it does not expose the wrapped reference unless it is restrained
///     by the lifetime of the token _and_ &mut XOR & remains enforced.
///   - As any token, it cannot be cloned.
///   - Due to behaving like a mutable reference to a primitive slice, the
///     buffer is [`Send`] and [`Sync`]. Using it on a different thread is safe
///     if it is returned to the allocator from the thread that allocated it
///     unless the allocator itself is thread safe.
///
/// The buffer represented by this token can be used to back zerocopy messages,
/// e.g. in the following use cases:
/// - The message can be sent over one or several channels with non-static
///   lifetime without ever having to copy it.
/// - The message can be converted between different representations without
///   copying the underlying buffer (e.g. to convert an MPDU to a low-level
///   driver frame and back)
/// - The message can be consumed and then re-instantiated from the same buffer
///   or a pointer derived from it due to the !Copy and "no-move" semantics
///   of this buffer.
#[derive(Debug, PartialEq, Eq)]
pub struct BufferToken(
    // Note: We keep a slice (fat pointer) rather than a reference to an array
    //       (thin pointer). This costs us extra bytes for a usize but allows us
    //       to allocate variable-sized buffers depending on the required buffer
    //       size w/o polluting generics.
    // Safety: static references never move, this allows us to safely convert
    //         this buffer to a pointer and back.
    &'static mut [u8],
    TokenGuard,
);

impl BufferToken {
    /// Creates a new buffer token.
    pub fn new(buffer: &'static mut [u8]) -> Self {
        Self(buffer, TokenGuard)
    }

    /// Consume the token.
    ///
    /// Safety: Must be called by the same allocator from which the buffer token
    ///         was allocated. Calling this from outside the allocator will leak
    ///         the buffer.
    pub unsafe fn consume(self) -> &'static mut [u8] {
        self.1.consume();
        self.0
    }
}

impl Deref for BufferToken {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl DerefMut for BufferToken {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0
    }
}

/// A utility communicating the intent that tokens _should_ be used as linear
/// types i.e. they must not be dropped unless explicitly consumed.
///
/// A token can still be leaked in several ways which would neutralize the drop
/// guard. That's ok. This pattern is not meant to be literally foolproof, just
/// to keep most users from accidentally doing the wrong thing in practice.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TokenGuard;

impl TokenGuard {
    /// Consumes the token.
    pub(crate) fn consume(self) {
        mem::forget(self);
    }
}

impl Drop for TokenGuard {
    fn drop(&mut self) {
        panic!("Tokens must not be dropped. Always return them to the channel.")
    }
}

/// Futures cancellation guard.
#[must_use = "Must be inactivated when the future is not cancelled."]
pub struct CancellationGuard<F: FnMut()> {
    on_cancellation: F,
}

impl<F: FnMut()> CancellationGuard<F> {
    /// The given closure will be run when the guard is dropped before it was
    /// inactivated. This can be used to clean-up on cancellation of a future.
    pub fn new(on_cancellation: F) -> Self {
        Self { on_cancellation }
    }

    /// Prevent drop handler from running.
    pub fn inactivate(self) {
        mem::forget(self)
    }
}

impl<F: FnMut()> Drop for CancellationGuard<F> {
    fn drop(&mut self) {
        (self.on_cancellation)()
    }
}
