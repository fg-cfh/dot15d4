use core::{array::from_fn, task::Waker};

use bitmaps::{Bitmap, Bits, BitsImpl};
use heapless::Deque;

use crate::queue::DequeWrapper;

/// Wrapper to store both, requests and responses, within the same message slot.
pub(crate) enum Message<Request, Response> {
    /// A message representing a request.
    Request(Request),

    /// A message representing a response.
    Response(Response),
}

/// A trait to be implemented by requests. This trait enables routing of
/// requests to appropriate selectable receivers.
///
/// Receivers will be registered with a receiver address. A request will be
/// directed to the first receiver whose address matches the request address.
pub trait HasAddress<Address> {
    /// Return the address of the request.
    fn address(&self) -> Address;
}

/// MESSAGES defines the number of available slots for messages that may be sent
/// concurrently over the channel.
///
/// CONSUMERS defines the number of available slots for consumers that may be
/// concurrently listening for requests on the channel.
///
/// The lifecycle of an individual message slot:
/// - available
/// - allocated
/// - pending
/// - receiving
/// - released (i.e. available again)
pub(crate) struct State<
    Address: PartialEq + Clone,
    Request: HasAddress<Address>,
    Response,
    const MESSAGES: usize,
    const CONSUMERS: usize,
> where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    /// Pre-allocated message slots. Messages are expected to be small.
    ///
    /// Large resources linked to messages SHALL be kept in separately allocated
    /// buffers.
    pub(crate) messages: [Option<Message<Request, Response>>; MESSAGES],

    /// Contains the list of slots with pending requests in the order they
    /// became pending.
    ///
    /// Safety: This is a non-synchronized queue. Unfortunately we cannot use
    ///         the synchronized [`heapless::spsc::Queue`] for now as its
    ///         capacity is N-1 which unnecessarily complicates the required
    ///         const generic arguments.
    pub(crate) pending_requests: Deque<u8, MESSAGES>,

    /// When a request becomes pending, the first consumer selectable by the
    /// requests's address will be woken and receives the request.
    ///
    /// The consumer address may be a wildcard address matching all requests or
    /// a well-defined selection. A corresponding [`PartialEq`] implementation
    /// must be given for the address space.
    ///
    /// Note: Currently we assume that the consumer list is short. Therefore
    ///       iteratively O(n)-searching for matching consumers is less resource
    ///       intensive than using a hash map, binary search or similar.
    ///
    /// Note: Requests are not cloneable in general. As we hand out ownership of
    ///       requests to consumers, only a single consumer should currently be
    ///       matching per request. This might change in the future: We may
    ///       introduce an order to consumers and re-define the consumer list as
    ///       a "filter onion" such that each consumer may individually decide
    ///       whether they consume a request (OK + Response), drop it (DROP) or
    ///       pass it on to consumers further down the chain without consuming
    ///       it themselves (CONTINUE).
    pub(crate) consumers: [Option<(Address, Waker)>; CONSUMERS],

    /// A bitmap that manages message slots: 0 - in use, 1 - available
    msg_available: Bitmap<MESSAGES>,

    /// A bitmap that manages consumer slots: 0 - in use, 1 - available
    cons_available: Bitmap<CONSUMERS>,
}

/// Safety: None of the methods are idempotent. They must not be called from
///         call-sites prone to spurious wake-ups (e.g. the pending branch of a
///         poll function).
impl<
        Address: PartialEq + Clone,
        Request: HasAddress<Address>,
        Response,
        const MESSAGES: usize,
        const CONSUMERS: usize,
    > State<Address, Request, Response, MESSAGES, CONSUMERS>
where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    pub(crate) fn new() -> Self {
        assert!(MESSAGES <= u8::MAX as _, "producers > 256");
        Self {
            messages: from_fn(|_| None),
            msg_available: Bitmap::mask(MESSAGES),
            pending_requests: Deque::new(),
            consumers: from_fn(|_| None),
            cons_available: Bitmap::mask(CONSUMERS),
        }
    }

    /// Guarantees exclusive access to the returned message slot in the channel.
    pub(crate) fn allocate_msg_slot(&mut self) -> Option<u8> {
        match self.msg_available.first_index() {
            None => None,
            Some(msg_slot) => {
                self.msg_available.set(msg_slot, false);
                Some(msg_slot as u8)
            }
        }
    }

    /// Return a message slot to the list of available slots.
    ///
    /// Safety: The slot must still be allocated (= not available) but no longer
    ///         pending.
    pub(crate) fn release_msg_slot(&mut self, msg_slot: u8) {
        let was_available = self.msg_available.set(msg_slot as _, true);
        debug_assert!(!was_available);
    }

    /// Guarantees exclusive access to the returned consumer slot in the
    /// channel.
    pub(crate) fn allocate_cons_slot(&mut self) -> Option<u8> {
        match self.cons_available.first_index() {
            None => None,
            Some(const_slot) => {
                self.cons_available.set(const_slot, false);
                Some(const_slot as u8)
            }
        }
    }

    /// Return a consumer slot to the list of available slots.
    ///
    /// Safety: The slot must still be allocated.
    pub(crate) fn release_cons_slot(&mut self, cons_slot: u8) {
        let was_available = self.cons_available.set(cons_slot as _, true);
        debug_assert!(!was_available);
    }

    /// Store the request, mark the slot as pending and notify the first
    /// matching consumer (if any) that a request is pending.
    pub(crate) fn send(&mut self, msg_slot: u8, request: Request) {
        // Safety: Slot must be reserved (= not available) but not yet pending.
        debug_assert!(!self.is_msg_slot_available(msg_slot));
        debug_assert!(self.messages[msg_slot as usize].is_none());

        let request_address = request.address();

        // Safety: A slot can never be allocated and pending at the same time.
        //         We're guaranteed exclusive access to the slot right now and
        //         may write to it. An available slot must be empty.
        self.messages[msg_slot as usize] = Some(Message::Request(request));

        // Safety: The queue has dedicated capacity for all slots.
        self.pending_requests.push_back(msg_slot).unwrap();

        // Wake the first matching consumer (if any).
        for consumer in &mut self.consumers {
            if let Some((consumer_address, _)) = consumer {
                if *consumer_address == request_address {
                    consumer
                        .take()
                        .map(|(_, consumer_waker)| consumer_waker.wake());
                    break;
                }
            }
        }
    }

    /// Try to find a pending request that matches the given address. If found,
    /// it will be removed from the list of pending requests and returned.
    pub(crate) fn try_receive(&mut self, address: &Address) -> Option<(u8, Request)> {
        for (index, &msg_slot) in self.pending_requests.iter().enumerate() {
            // Check whether this consumer listens for the pending request.
            match &self.messages[msg_slot as usize] {
                Some(Message::Request(request)) => {
                    if *address == request.address() {
                        // Safety: A slot can never be allocated and pending at
                        //         the same time. We're guaranteed exclusive access
                        //         to the slot right now and may write to it. A
                        //         pending slot must have been allocated and set.
                        if let Message::Request(request) =
                            self.messages[msg_slot as usize].take().unwrap()
                        {
                            // Remove the pending request from the list.
                            DequeWrapper::new(&mut self.pending_requests).remove(index);

                            return Some((msg_slot, request));
                        } else {
                            unreachable!()
                        }
                    }
                }
                _ => unreachable!(),
            }
        }

        None
    }

    /// Checks whether the message slot with the given slot id is available.
    fn is_msg_slot_available(&mut self, msg_slot: u8) -> bool {
        self.msg_available.get(msg_slot as _)
    }
}
