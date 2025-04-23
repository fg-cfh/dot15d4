use core::cell::{RefCell, RefMut};

use bitmaps::{Bits, BitsImpl};

use crate::{
    receiver::InternalReceiverApi,
    state::{HasAddress, State},
    tokens::{RequestToken, ResponseToken},
    Receiver,
};

/// A synchronous bounded queue for sending requests from multiple asynchronous
/// tasks to selectable receiving tasks with backpressure.
///
/// The channel will buffer requests up to the guaranteed capacity. Attempts to
/// allocate further message slots will fail.
///
/// Requests will be delivered to the receiver in the same order as they were
/// sent.
pub struct SyncChannel<
    Address: PartialEq + Clone,
    Request: HasAddress<Address>,
    Response, // Kept so that the receiver can be channel-type agnostic.
    const MESSAGES: usize,
    const CONSUMERS: usize,
> where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    state: RefCell<State<Address, Request, Response, MESSAGES, CONSUMERS>>,
}

impl<
        Address: PartialEq + Clone,
        Request: HasAddress<Address>,
        Response,
        const MESSAGES: usize,
        const CONSUMERS: usize,
    > SyncChannel<Address, Request, Response, MESSAGES, CONSUMERS>
where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    /// Initialize a new [`SyncChannel`].
    pub fn new() -> Self {
        Self {
            state: RefCell::new(State::new()),
        }
    }

    /// Returns an additional [`SyncSender`] attached to the channel.
    pub fn sender(&self) -> SyncSender<Address, Request, Response, MESSAGES, CONSUMERS> {
        SyncSender { channel: self }
    }

    /// Returns an additional [`Receiver`] attached to the channel.
    pub fn receiver(&self) -> Receiver<Address, Request, Response, MESSAGES, CONSUMERS, Self> {
        Receiver::new(self)
    }
}

/// Synchronous send-only access to a [`SyncChannel`].
pub struct SyncSender<
    'a,
    Address: PartialEq + Clone,
    Request: HasAddress<Address>,
    Response,
    const MESSAGES: usize,
    const CONSUMERS: usize,
> where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    channel: &'a SyncChannel<Address, Request, Response, MESSAGES, CONSUMERS>,
}

impl<
        'a,
        Address: PartialEq + Clone,
        Request: HasAddress<Address>,
        Response,
        const MESSAGES: usize,
        const CONSUMERS: usize,
    > SyncSender<'a, Address, Request, Response, MESSAGES, CONSUMERS>
where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    /// Tries to allocate a message slot.
    ///
    /// Changes the state of any allocated slot from available to allocated.
    pub fn try_allocate_request_token(&self) -> Option<RequestToken> {
        match self.channel.state.borrow_mut().allocate_msg_slot() {
            Some(msg_slot) => Some(RequestToken::new(msg_slot)),
            None => None,
        }
    }

    /// Synchronously sends the given message over a previously allocated slot
    /// (i.e. makes it "pending" on the receiver side).
    ///
    /// The method returns immediately ("fire and forget"). The message has not
    /// been delivered yet when this method returns. Does not support responses.
    ///
    /// Changes the state of the allocated slot from available to pending.
    ///
    /// Note: This operation cannot fail.
    pub fn send_request(&self, request_token: RequestToken, request: Request) {
        self.channel
            .state
            .borrow_mut()
            .send(request_token.consume(), request);
    }
}

impl<
        Address: PartialEq + Clone,
        Request: HasAddress<Address>,
        Response,
        const MESSAGES: usize,
        const CONSUMERS: usize,
    > InternalReceiverApi<Address, Request, Response, MESSAGES, CONSUMERS>
    for SyncChannel<Address, Request, Response, MESSAGES, CONSUMERS>
where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    fn received(&self, response_token: ResponseToken, _response: Response) {
        // Note: The response is silently dropped as the sync API doesn't
        //       support response delivery.
        self.state
            .borrow_mut()
            .release_msg_slot(response_token.consume());
    }

    fn state(&self) -> RefMut<State<Address, Request, Response, MESSAGES, CONSUMERS>> {
        self.state.borrow_mut()
    }
}
