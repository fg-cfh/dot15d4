use core::{
    cell::RefMut,
    future::{poll_fn, Future},
    marker::PhantomData,
    task::Poll,
};

use bitmaps::{Bits, BitsImpl};

use crate::{
    state::{HasAddress, State},
    tokens::{CancellationGuard, ConsumerToken, ResponseToken},
};

/// Consumer-side access to a channel.
#[allow(private_bounds)]
pub struct Receiver<
    'channel,
    Address: PartialEq + Clone,
    Request: HasAddress<Address>,
    Response,
    const MESSAGES: usize,
    const CONSUMERS: usize,
    Channel: InternalReceiverApi<Address, Request, Response, MESSAGES, CONSUMERS>,
> where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    channel: &'channel Channel,
    address: PhantomData<Address>,
    request: PhantomData<Request>,
    response: PhantomData<Response>,
}

#[allow(private_bounds)]
impl<
        'a,
        Address: PartialEq + Clone,
        Request: HasAddress<Address>,
        Response,
        const MESSAGES: usize,
        const CONSUMERS: usize,
        Channel: InternalReceiverApi<Address, Request, Response, MESSAGES, CONSUMERS>,
    > Receiver<'a, Address, Request, Response, MESSAGES, CONSUMERS, Channel>
where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    /// Instantiate a new receiver for the given channel.
    pub fn new(channel: &'a Channel) -> Self {
        Self {
            channel,
            address: PhantomData,
            request: PhantomData,
            response: PhantomData,
        }
    }

    /// Try to allocate a consumer token.
    ///
    /// Consumers typically acquire a consumer token once when the consumer task
    /// starts and then present it on any reception.
    ///
    /// A consumer may allocate more than one consumer token to be able to
    /// receive requests in parallel.
    pub fn try_allocate_consumer_token(&self) -> Option<ConsumerToken> {
        self.channel.try_allocate_consumer_token()
    }

    /// Release a consumer token.
    ///
    /// Consumers typically call this method just before the consumer task ends.
    ///
    /// Calling this method will release a previously allocated consumer slot
    /// back to the pool.
    pub fn release_consumer_token(&self, consumer_token: ConsumerToken) {
        self.channel.release_consumer_token(consumer_token)
    }

    /// Wait for a request matching the given address or address wildcard and
    /// return it together with a token representing the message slot allocated
    /// for the response.
    ///
    /// Reception occurs in the order that requests became pending.
    ///
    /// If several receivers match a pending request, only a single, arbitrary
    /// one will receive the request.
    ///
    /// Note: Other than on the producer side (which may have to wait for a
    ///       radio slot to become available) we do not allow for backpressure
    ///       on the consumer side as we currently assume that local consumers
    ///       should be fast enough to handle reception synchronously. Therefore
    ///       it is required to present an allocated consumer token to get
    ///       access to the channel.
    ///
    /// On return, changes the state of the matching message slot from pending
    /// to receiving.
    ///
    /// Canceling reception will free the consumer slot.
    pub async fn wait_for_request(
        &self,
        consumer_token: &mut ConsumerToken,
        address: &Address,
    ) -> (ResponseToken, Request) {
        self.channel.wait_for_request(consumer_token, address).await
    }

    /// Try to receive a pending request matching the given address or address
    /// wildcard and return it together with a token representing the message
    /// slot allocated for the response.
    ///
    /// The same ordering, filtering and backpressure rules apply as for
    /// [`Self::wait_for_request()`], see there.
    ///
    /// Changes the state of any matching message slot from pending to
    /// receiving.
    pub fn try_receive_request(
        &self,
        consumer_token: &mut ConsumerToken,
        address: &Address,
    ) -> Option<(ResponseToken, Request)> {
        self.channel.try_receive_request(consumer_token, address)
    }

    /// Releases the message slot for re-use and signals delivery to
    /// asynchronous senders returning a response.
    ///
    /// Defining the notion of "delivery" is up to the implementor. It can
    /// signal delivery of a message over the air, a promise that certain
    /// resources have been released, reception of a response by a peer, etc.
    ///
    /// Changes the state of the allocated message slot from receiving to
    /// released (available).
    ///
    /// Note: Not calling this method will block the slot forever.
    pub fn received(&self, response_token: ResponseToken, response: Response) {
        self.channel.received(response_token, response)
    }

    /// Convenience method over `wait_for_msg()` and `receive_done()`.
    pub async fn receive<
        Result,
        Fut: Future<Output = (Response, Result)>,
        F: FnOnce(Request) -> Fut,
    >(
        &self,
        consumer_token: &mut ConsumerToken,
        address: &Address,
        f: F,
    ) -> Result {
        let (response_token, request) = self.wait_for_request(consumer_token, address).await;
        let (response, result) = f(request).await;
        self.received(response_token, response);
        result
    }
}

pub(crate) trait InternalReceiverApi<
    Address: PartialEq + Clone,
    Request: HasAddress<Address>,
    Response,
    const MESSAGES: usize,
    const CONSUMERS: usize,
> where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    /// See [`Receiver::try_allocate_consumer_token()`]
    fn try_allocate_consumer_token(&self) -> Option<ConsumerToken> {
        self.state()
            .allocate_cons_slot()
            .map(|consumer_slot| ConsumerToken::new(consumer_slot))
    }

    /// See [`InternalReceiverApi::release_consumer_token()`]
    fn release_consumer_token(&self, consumer_token: ConsumerToken) {
        self.state().release_cons_slot(consumer_token.consume());
    }

    /// See [`Receiver::wait_for_request()`]
    fn wait_for_request(
        &self,
        consumer_token: &mut ConsumerToken, // Mutability guarantees exclusive access.
        address: &Address,
    ) -> impl Future<Output = (ResponseToken, Request)> {
        let consumer_slot = consumer_token.consumer_slot();

        async move {
            let cancellation_guard = CancellationGuard::new(|| {
                // Clean up the consumer slot.
                self.state().consumers[consumer_slot as usize] = None;
            });

            let result = poll_fn(move |cx| {
                let mut state = self.state();
                match state.try_receive(address) {
                    Some((msg_slot, request)) => {
                        Poll::Ready((ResponseToken::new(msg_slot), request))
                    }
                    None => {
                        // None of the pending messages fits the given address, so let's
                        // wait for one that fits.
                        debug_assert!(state.consumers[consumer_slot as usize].is_none());
                        state.consumers[consumer_slot as usize] =
                            Some((address.clone(), cx.waker().clone()));
                        Poll::Pending
                    }
                }
            })
            .await;

            cancellation_guard.inactivate();

            result
        }
    }

    /// See [`Receiver::try_receive_request()`]
    fn try_receive_request(
        &self,
        _consumer_token: &mut ConsumerToken,
        address: &Address,
    ) -> Option<(ResponseToken, Request)> {
        self.state()
            .try_receive(address)
            .map(|(msg_slot, request)| (ResponseToken::new(msg_slot), request))
    }

    /// See [`Receiver::received()`]
    fn received(&self, response_token: ResponseToken, response: Response);

    /// Get the internal state of the receiving channel.
    fn state(&self) -> RefMut<State<Address, Request, Response, MESSAGES, CONSUMERS>>;
}
