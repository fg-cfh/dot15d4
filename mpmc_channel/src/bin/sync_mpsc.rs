#![cfg_attr(not(feature = "std"), no_std, no_main)]

use core::pin::Pin;
use embassy_executor::Spawner;
use embassy_time::Timer;
#[cfg(feature = "std")]
use log::info;
use mpmc_channel::{
    BufferAllocator, BufferAllocatorBackend, BufferToken, HasAddress, Receiver, SyncChannel,
    SyncSender,
};
use static_cell::StaticCell;

const NUM_PRODUCERS: usize = 5;
const CHANNEL_CAPACITY: usize = NUM_PRODUCERS;
const NUM_CONSUMERS: usize = 1;

const MESSAGE_BUFFER_SIZE: usize = 3;
const NUM_MESSAGE_BUFFERS: usize = CHANNEL_CAPACITY;

struct Request {
    buffer: BufferToken,
}

impl HasAddress<()> for Request {
    // In the MPSC case the address can be anything that is unique, even a
    // zero-sized object.
    fn address(&self) -> () {}
}

type SyncMpscChannel = SyncChannel<(), Request, (), CHANNEL_CAPACITY, NUM_CONSUMERS>;
type SyncMpscSender = SyncSender<'static, (), Request, (), CHANNEL_CAPACITY, NUM_CONSUMERS>;
type MpscReceiver =
    Receiver<'static, (), Request, (), CHANNEL_CAPACITY, NUM_CONSUMERS, SyncMpscChannel>;

#[embassy_executor::task(pool_size = NUM_PRODUCERS)]
async fn producer(id: u8, buffer_allocator: BufferAllocator, sender: SyncMpscSender) {
    let mut counter = 0;
    loop {
        if let Some(request_token) = sender.try_allocate_request_token() {
            let msg_slot_id = request_token.message_slot();
            #[cfg(feature = "std")]
            info!("producer {id}: sending {counter} over {msg_slot_id}");

            // Safety: We have a dedicated buffer per slot and the receiver
            //         ensures that it will be released before the producer
            //         gains back control. Buffers must be allocated _after_ a
            //         message slot has already been allocated to avoid
            //         deadlock.
            let mut buffer = buffer_allocator
                .try_allocate_buffer(MESSAGE_BUFFER_SIZE)
                .expect("out of memory");
            buffer.copy_from_slice(&[id, msg_slot_id, counter]);

            let request = Request { buffer };
            sender.send_request(request_token, request);

            counter = counter.wrapping_add(1);
            Timer::after_millis(100).await;
        } else {
            // Spinning: Fairness is not guaranteed. Synchronous clients
            // requiring fairness need some kind of out-of-band co-ordination
            // among themselves.
            Timer::after_millis(10).await;
        }
    }
}

#[embassy_executor::task]
async fn consumer(buffer_allocator: BufferAllocator, receiver: MpscReceiver) {
    let mut consumer_token = receiver.try_allocate_consumer_token().unwrap();
    loop {
        match receiver.try_receive_request(&mut consumer_token, &()) {
            Some((response_token, request)) => {
                let buffer = request.buffer;

                // Long delivery delay to demonstrate backpressure.
                Timer::after_millis(1000).await;
                #[cfg(feature = "std")]
                info!(
                    "producer {}: received {} over {}",
                    buffer[0], buffer[2], buffer[1]
                );

                // Safety: buffers need to be explicitly released to (a copy of)
                //         the allocator from which they were allocated before
                //         they can be re-used.
                unsafe { buffer_allocator.deallocate_buffer(buffer) };

                receiver.received(response_token, ());
            }
            None => {
                Timer::after_millis(10).await;
            }
        }
    }
}

fn mpdu_channel() -> &'static SyncMpscChannel {
    // In the synchronous case, there is no backlog.
    static CHANNEL: StaticCell<SyncMpscChannel> = StaticCell::new();
    CHANNEL.init(SyncChannel::new())
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    #[cfg(feature = "std")]
    env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .format_timestamp_nanos()
        .init();

    type AllocatorBackend = BufferAllocatorBackend<MESSAGE_BUFFER_SIZE, NUM_MESSAGE_BUFFERS>;
    static ALLOCATOR_BACKEND: StaticCell<AllocatorBackend> = StaticCell::new();
    static ALLOCATOR: StaticCell<Pin<&'static AllocatorBackend>> = StaticCell::new();
    let buffer_allocator =
        BufferAllocator::new(ALLOCATOR.init(ALLOCATOR_BACKEND.init(Default::default()).pin()));

    let channel = mpdu_channel();
    for id in 0..NUM_PRODUCERS {
        spawner
            .spawn(producer(id as u8, buffer_allocator, channel.sender()))
            .unwrap();
    }
    spawner
        .spawn(consumer(buffer_allocator, channel.receiver()))
        .unwrap();
}
