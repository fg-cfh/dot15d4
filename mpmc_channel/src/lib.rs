//! A bounded queue for sending requests frk om multiple asynchronous tasks to a
//! one or many receiving task(s) with backpressure.
//!
//! The channel is not synchronized across threads. It can be used concurrently
//! by multiple producer (sender) and consumer (receiver) tasks as long as both,
//! the producers and the receivers, are managed by a single executor (thread).
//!
//! This module provides a bounded channel that has a limit on the number of
//! messages that it can store. If this limit is reached, trying to send another
//! message will exert backpressure on the sender.
//!
//! The channel exposes two different APIs to senders.
//!
//! 1. Synchronous API:
//!
//!   - [`SyncSender::try_allocate_request_token()]
//!     Returns a request token if a message slot is available, None otherwise.
//!
//!   - [`SyncSender::send_request()]
//!     Sends a request. This call is guaranteed to succeed. Returns immediately
//!     without delivery feedback.
//!
//!   This is a "fire and forget" API for synchronous producers. It is mainly
//!   used to support smoltcp's synchronous device abstraction. It currently
//!   silently drops responses as smoltcp cannot deal with asynchronous
//!   responses anyway.
//!
//! 2. Asynchronous API:
//!
//!   - [`AsyncSender::allocate_request_token()`]
//!     Waits until a request token becomes available and returns it.
//!
//!   - [`AsyncSender::send_request_and_wait()`]
//!     Consumes a request token to send the given request and waits until it
//!     was delivered to the receiver. Returns the response produced by the
//!     receiver.
//!
//!   - [`AsyncSender::send()`]
//!     A convenience method over [`AsyncSender::allocate_request_token()`] and
//!     [`AsyncSender::send_request_and_wait()`]
//!
//!   This API targets asynchronous producers, provides feedback about delivery,
//!   supports a request/response model as required by the IEEE 802.15.4
//!   standard and facilitates safe resource management.
//!
//! On the receiver side, a combined sync/async API is exposed:
//!   - [`Receiver::try_allocate_consumer_token()`]
//!     Tries to allocate a consumer token that allows a single consumer to
//!     listen for requests.
//!
//!   - [`Receiver::release_consumer_token()`]
//!     Releases a previously
//!
//!   - [`Receiver::wait_for_request()`]
//!     Asynchronously waits until a matching request becomes pending and
//!     returns it. Also returns the token required to signal delivery to the
//!     sender.
//!
//!   - [`Receiver::try_receive_request()`]
//!     Synchronously checks whether a matching request is pending. If so,
//!     returns the pending request together with the token required to signal
//!     delivery to the sender.
//!
//!   - [`Receiver::received()`]
//!     Releases the message slot and signals delivery to the sender.
//!
//!   - [`Receiver::receive()`]
//!     This is convenience method over [`Receiver::wait_for_request()`] and
//!     [`Receiver::received()`]. It handles message reception in a closure.

// TODO: Make a prioritized version of the channel for timestamped radio tasks
//       that allows awaiting changes to the queue front.

#![cfg_attr(not(feature = "std"), no_std)]

pub use allocator::{
    AsyncBufferAllocator, BufferAllocator, BufferAllocatorBackend, BufferAllocatorBacklog,
};
pub use async_channel::{AsyncChannel, AsyncSender};
pub use receiver::Receiver;
pub use state::HasAddress;
pub use sync_channel::{SyncChannel, SyncSender};
pub use tokens::{BufferToken, CancellationGuard};

// Re-exports required in macros.
pub mod export {
    pub use allocator_api2::alloc::Allocator;
    pub use static_cell::StaticCell;
}

mod allocator;
mod async_channel;
mod queue;
mod receiver;
mod state;
mod sync_channel;
mod tokens;
