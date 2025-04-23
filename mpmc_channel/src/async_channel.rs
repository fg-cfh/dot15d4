use core::{
    array::from_fn,
    cell::{RefCell, RefMut},
    future::{poll_fn, Future},
    task::{Poll, Waker},
};

use bitmaps::{Bits, BitsImpl};
use heapless::Deque;

use crate::{
    receiver::InternalReceiverApi,
    state::{HasAddress, Message, State},
    tokens::{CancellationGuard, RequestToken, ResponseToken},
    Receiver,
};

/// An asynchronous bounded MPMC queue for sending requests from multiple
/// asynchronous producer tasks to selectable receiving tasks with backpressure.
///
/// The channel will buffer requests up to the guaranteed capacity and will then
/// be able to backlog a limited number of further producer tasks while they are
/// waiting for a message slot to become available. Trying to schedule waiting
/// producer tasks beyond the capacity of the backlog will cause the queue to
/// panic.
///
/// More specifically: Given `PRODUCERS` as the number of independent tasks that
/// are accessing the queue in parallel and `MESSAGES` as the number of messages
/// that can be handled concurrently, the `BACKLOG` parameter needs to be set to
/// `PRODUCERS - MESSAGES` for panic-free queue operation.
///
/// Requests will be delivered to the receiver in the same order as they were
/// sent.
pub struct AsyncChannel<
    Address: PartialEq + Clone,
    Request: HasAddress<Address>,
    Response,
    const MESSAGES: usize,
    const BACKLOG: usize,
    const CONSUMERS: usize,
> where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    state: RefCell<State<Address, Request, Response, MESSAGES, CONSUMERS>>,
    async_state: RefCell<AsyncState<MESSAGES, BACKLOG>>,
}

impl<
        Address: PartialEq + Clone,
        Request: HasAddress<Address>,
        Response,
        const MESSAGES: usize,
        const BACKLOG: usize,
        const CONSUMERS: usize,
    > AsyncChannel<Address, Request, Response, MESSAGES, BACKLOG, CONSUMERS>
where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    /// Initialize a new [`AsyncChannel`].
    pub fn new() -> Self {
        Self {
            state: RefCell::new(State::new()),
            async_state: RefCell::new(AsyncState::new()),
        }
    }

    /// Returns an additional [`AsyncSender`] attached to the channel.
    pub fn sender(&self) -> AsyncSender<Address, Request, Response, MESSAGES, BACKLOG, CONSUMERS> {
        AsyncSender { channel: self }
    }

    /// Returns an additional [`Receiver`] attached to the channel.
    pub fn receiver(&self) -> Receiver<Address, Request, Response, MESSAGES, CONSUMERS, Self> {
        Receiver::new(self)
    }
}

enum DeliveryState {
    NotSent,
    Sent(Waker),
    Delivered,
}

/// Asynchronous send-only access to a [`AsyncChannel`].
pub struct AsyncSender<
    'a,
    Address: PartialEq + Clone,
    Request: HasAddress<Address>,
    Response,
    const MESSAGES: usize,
    const BACKLOG: usize,
    const CONSUMERS: usize,
> where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    channel: &'a AsyncChannel<Address, Request, Response, MESSAGES, BACKLOG, CONSUMERS>,
}

impl<
        'a,
        Address: PartialEq + Clone,
        Request: HasAddress<Address>,
        Response,
        const MESSAGES: usize,
        const BACKLOG: usize,
        const CONSUMERS: usize,
    > AsyncSender<'a, Address, Request, Response, MESSAGES, BACKLOG, CONSUMERS>
where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    /// Waits until a message slot becomes available and blocks it's capacity
    /// for later use by the sender.
    ///
    /// Changes the state of the allocated slot from available to allocated.
    ///
    /// Note: Using this method introduces a risk of deadlock unless you ensure
    ///       that at least one task owning and willing to release a request
    ///       token is life when blocking on this method. See
    ///       [`crate::allocator::AsyncBufferAllocator::allocate_buffer()`] for
    ///       more information and an example.
    pub fn allocate_request_token(
        &self,
    ) -> impl Future<Output = RequestToken>
           + use<'a, Address, Request, Response, MESSAGES, BACKLOG, CONSUMERS> {
        poll_fn(|cx| {
            let state = &mut self.channel.state.borrow_mut();
            let async_state = &mut self.channel.async_state.borrow_mut();
            match state.allocate_msg_slot() {
                Some(msg_slot) => {
                    async_state.msg_slot_allocated(msg_slot);
                    Poll::Ready(RequestToken::new(msg_slot))
                }
                None => {
                    async_state.msg_slot_unavailable(cx.waker().clone());
                    Poll::Pending
                }
            }
        })
    }

    /// Sends the given request and then waits until it has been delivered and
    /// the message slot was released.
    ///
    /// Changes the state of the allocated message slot from available to
    /// pending and waits until it is being released (i.e. becomes available
    /// again).
    ///
    /// Panics when cancelled.
    ///
    /// Note: As we hand over ownership of requests on delivery, their content
    ///       (including any references to buffers or other dependent resources)
    ///       may leak beyond the time of return of this method. You may not
    ///       assume that resources assigned to the request can be re-used
    ///       unless you implement drop handlers that allow you to prove that
    ///       dependent resources blocked by the message have actually been
    ///       released.
    pub async fn send_request_and_wait(
        &self,
        request_token: RequestToken,
        request: Request,
    ) -> Response {
        let msg_slot = request_token.consume();
        self.channel.state.borrow_mut().send(msg_slot, request);

        let cancellation_guard = CancellationGuard::new(|| panic!("cancelled"));

        let result = poll_fn(move |cx| {
            let state = &mut self.channel.state.borrow_mut();
            let async_state = &mut self.channel.async_state.borrow_mut();

            let delivery_state = &mut async_state.delivery_state[msg_slot as usize];
            if let DeliveryState::Delivered = delivery_state {
                *delivery_state = DeliveryState::NotSent;

                let response = if let Message::Response(response) =
                    // Safety: We were woken because the receiver acknowledged
                    //         delivery with a response (see delivery_state
                    //         above). Therefore a response must be present in
                    //         the allocated message slot by now.
                    state.messages[msg_slot as usize].take().unwrap()
                {
                    response
                } else {
                    unreachable!()
                };

                state.release_msg_slot(msg_slot);
                async_state.msg_slot_now_available();

                Poll::Ready(response)
            } else {
                *delivery_state = DeliveryState::Sent(cx.waker().clone());
                Poll::Pending
            }
        })
        .await;

        cancellation_guard.inactivate();

        result
    }

    /// Convenience method that allocates a message slot, sends the given
    /// message as soon as a slot becomes available and then waits until the
    /// message has been delivered.
    pub async fn send(&self, request: Request) -> Response {
        let request_token = self.allocate_request_token().await;
        self.send_request_and_wait(request_token, request).await
    }
}

impl<
        Address: PartialEq + Clone,
        Request: HasAddress<Address>,
        Response,
        const MESSAGES: usize,
        const BACKLOG: usize,
        const CONSUMERS: usize,
    > InternalReceiverApi<Address, Request, Response, MESSAGES, CONSUMERS>
    for AsyncChannel<Address, Request, Response, MESSAGES, BACKLOG, CONSUMERS>
where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    fn received(&self, response_token: ResponseToken, response: Response) {
        let msg_slot = response_token.consume();

        // Signal to the sending task, that a the message in this slot has
        // been fully received.
        let mut state = self.state.borrow_mut();
        if let DeliveryState::Sent(waker) = core::mem::replace(
            &mut self.async_state.borrow_mut().delivery_state[msg_slot as usize],
            DeliveryState::Delivered,
        ) {
            debug_assert!(state.messages[msg_slot as usize].is_none());
            state.messages[msg_slot as usize] = Some(Message::Response(response));
            waker.wake();
        } else {
            // Safety: For the asynchronous interface, a waker must have been
            //         registered.
            unreachable!()
        }
    }

    fn state(&self) -> RefMut<State<Address, Request, Response, MESSAGES, CONSUMERS>> {
        self.state.borrow_mut()
    }
}

/// MESSAGES is the intended capacity of the channel. BACKLOG is the number of
/// producers that may wait for a slot to become available.
struct AsyncState<const MESSAGES: usize, const BACKLOG: usize>
where
    BitsImpl<MESSAGES>: Bits,
{
    /// Woken when the request in a given slot has been delivered.
    delivery_state: [DeliveryState; MESSAGES],

    /// Contains the list of tasks waiting for a message slot in the order they
    /// started waiting.
    ///
    /// The tasks are woken in the order they started waiting as soon as a slot
    /// becomes available.
    backlog: Deque<Waker, BACKLOG>,
}

/// Safety: None of the methods are idempotent. They must not be called from
///         call-sites prone to spurious wake-ups (e.g. the pending branch of a
///         poll function).
impl<const MESSAGES: usize, const BACKLOG: usize> AsyncState<MESSAGES, BACKLOG>
where
    BitsImpl<MESSAGES>: Bits,
{
    fn new() -> Self {
        Self {
            delivery_state: from_fn(|_| DeliveryState::NotSent),
            backlog: Deque::new(),
        }
    }

    fn msg_slot_allocated(&mut self, slot: u8) {
        self.delivery_state[slot as usize] = DeliveryState::NotSent;
    }

    fn msg_slot_unavailable(&mut self, waker: Waker) {
        self.backlog.push_front(waker).expect("backlog full");
    }

    fn msg_slot_now_available(&mut self) {
        // Signal to the next task waiting for slots (if any), that a slot is
        // now available.
        self.backlog.pop_back().map(|waker| waker.wake());
    }
}
