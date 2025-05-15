pub mod acknowledgment;
pub mod constants;
pub mod mcps;
pub mod mlme;
pub mod neighbors;
pub mod pib;
pub mod primitives;
pub mod tsch;
pub mod utils;

use core::cell::{Cell, RefCell};

use crate::{
    radio::{RadioTask, RadioTaskResponse, RadioTaskSender, DRIVER_CHANNEL_CAPACITY},
    sync::{mutex::Mutex, select, Either},
};
use dot15d4_frame3::{
    driver::{
        DriverConfig, DroppableRadioFrame, RadioFrame, RadioFrameRepr, RadioFrameSized,
        RadioFrameUnsized,
    },
    frame_control::FrameType,
    mpdu::MpduFrame,
};
use embedded_hal_async::delay::DelayNs;
use mpmc_channel::{AsyncBufferAllocator, AsyncSender, Receiver, SyncChannel, SyncSender};
use pib::Pib;
use primitives::{MacIndication, MacRequest};
use rand_core::RngCore;

#[cfg(feature = "rtos-trace")]
use crate::trace::{MAC_INDICATION, MAC_REQUEST};

// TODO: Make channel capacities and the number of upper layer tasks
//       configurable.

/// The number of MAC requests that may be pending or executing in parallel.
///
/// Note: In TSCH MAC requests may overlap both, between peers and between
///       parallel tasks (e.g. data and control requests). Therefore, we usually
///       will require a capacity greater than one.
const UL_NUM_PARALLEL_REQUESTS: usize = 1;

/// The number of upper layer tasks that receive indications independently from
/// each other.
///
/// Note: Currently we assume a single data channel terminated by a smoltcp
///       client. But this may change once we expose one or more independent
///       control channels towards applications directly.
///
/// Note: We currently assume that tasks will handle indications sequentially.
///       Therefore only a single indication may be pending per upper layer
///       task.
const UL_NUM_TASKS: usize = 1;
const UL_NUM_PARALLEL_INDICATIONS: usize = UL_NUM_TASKS;
const UL_INDICATION_BACKLOG: usize = 0;

pub type MacRequestChannel = SyncChannel<(), MacRequest, (), UL_NUM_PARALLEL_REQUESTS, 1>;
pub type MacRequestReceiver<'channel> =
    Receiver<'channel, (), MacRequest, (), UL_NUM_PARALLEL_REQUESTS, 1, MacRequestChannel>;
pub type MacRequestSender<'channel> =
    SyncSender<'channel, (), MacRequest, (), UL_NUM_PARALLEL_REQUESTS, 1>;

pub type MacIndicationChannel = SyncChannel<(), MacIndication, (), UL_NUM_TASKS, UL_NUM_TASKS>;
pub type MacIndicationReceiver<'channel> = Receiver<
    'channel,
    (),
    MacIndication,
    (),
    UL_NUM_PARALLEL_INDICATIONS,
    UL_NUM_TASKS,
    MacIndicationChannel,
>;
pub type MacIndicationSender<'channel> = AsyncSender<
    'channel,
    (),
    MacIndication,
    (),
    UL_NUM_PARALLEL_INDICATIONS,
    UL_INDICATION_BACKLOG,
    UL_NUM_TASKS,
>;

// TODO: Challenge the following capacity calculation.
/// The following worst-case assumptions underlie our current buffer allocator
/// capacity calculation:
/// - The buffer allocator has a capacity of at least one buffer.
/// - We need at most as many buffers as we allocate message capacity on all channels.
const MIN_AVAILABLE_BUFFERS: usize = 1;
const MAX_REQUESTED_BUFFERS: usize =
    UL_NUM_PARALLEL_REQUESTS + UL_NUM_PARALLEL_INDICATIONS + DRIVER_CHANNEL_CAPACITY;
/// Currently only an ACK buffer is pre-allocated by the MAC service.
const NUM_PREALLOCATED_BUFFERS: usize = 1;
const BUFFER_BACKLOG: usize =
    MAX_REQUESTED_BUFFERS - MIN_AVAILABLE_BUFFERS + NUM_PREALLOCATED_BUFFERS;
pub type MacBufferAllocator = AsyncBufferAllocator<BUFFER_BACKLOG>;

#[allow(dead_code)]
/// A structure exposing MAC sublayer services such as MLME and MCPS. This runs
/// the main event loop that handles interactions between an upper layer and the
/// PHY sublayer. It uses channels to communicate with upper layer tasks and
/// with radio drivers.
pub struct MacService<'svc, Rng: RngCore, TIMER, Config: DriverConfig> {
    /// Pseudo-random number generator
    rng: &'svc mut Mutex<Rng>,
    /// Timer enabling delays operation
    timer: TIMER,
    /// Message buffer allocator
    buffer_allocator: MacBufferAllocator,
    /// Upper layer channel from which MAC requests are received.
    request_receiver: MacRequestReceiver<'svc>,
    /// Upper layer channel to which MAC indications are sent.
    indication_sender: MacIndicationSender<'svc>,
    /// Channel to communicate with one or several radio drivers.
    radio_task_sender: RadioTaskSender<'svc, Config>,
    /// A pre-allocated frame for outgoing ACKs.
    tx_ack_frame: Cell<Option<RadioFrame<Config, RadioFrameSized>>>,
    /// PAN Information Base
    pib: RefCell<Pib>,
}

impl<'svc, Rng: RngCore, TIMER: DelayNs + Clone, Config: DriverConfig>
    MacService<'svc, Rng, TIMER, Config>
{
    /// Creates a new [`MacService<Rng, U, TIMER, R>`].
    pub fn new(
        rng: &'svc mut Mutex<Rng>,
        timer: TIMER,
        buffer_allocator: MacBufferAllocator,
        request_receiver: MacRequestReceiver<'svc>,
        indication_sender: MacIndicationSender<'svc>,
        radio_task_sender: RadioTaskSender<'svc, Config>,
    ) -> Self {
        // The outgoing ACK frame can be pre-allocated and pre-populated so that
        // we can re-use it with minimal runtime overhead across ACKs.
        //
        // Safety: We have separate incoming and outgoing ACK buffers to ensure
        //         that incoming ACKs cannot corrupt the pre-populated outgoing
        //         ACK buffer. This allows us to re-use the outgoing ACK buffer
        //         w/o validation.
        let tx_ack_frame = Cell::new(Some(Self::allocate_tx_ack(buffer_allocator)));

        Self {
            rng,
            timer,
            buffer_allocator,
            request_receiver,
            indication_sender,
            radio_task_sender,
            tx_ack_frame,
            pib: RefCell::new(Pib::default()),
        }
    }
}

#[allow(dead_code)]
impl<'svc, Rng: RngCore, TIMER: DelayNs + Clone, Config: DriverConfig>
    MacService<'svc, Rng, TIMER, Config>
{
    /// Run the main event loop used by the MAC sublayer for its operation. For
    /// now, the loop waits for either receiving a MCPS-DATA request from the
    /// upper layer or a radio frame from the driver.
    pub async fn run(&mut self) -> ! {
        let mut consumer_token = self
            .request_receiver
            .try_allocate_consumer_token()
            .expect("no capacity");
        let rx_buffer_size =
            RadioFrameRepr::<Config, RadioFrameUnsized>::new().max_buffer_length() as usize;

        loop {
            let mac_request_future = self
                .request_receiver
                .wait_for_request(&mut consumer_token, &());
            let rx_frame_future = self.radio_recv(rx_buffer_size);

            // Wait until we either have a request to process from the upper layer or we
            // receive an indication from the PHY sublayer
            // TODO: Implement a version of "select" that takes mutable
            //       references to futures rather than dropping the pending one.
            //       This is important so that an ongoing Rx task is not being
            //       cancelled and the radio put into idle mode each time a new
            //       request comes in.
            match select::select(mac_request_future, rx_frame_future).await {
                Either::First((response_token, mac_request)) => {
                    #[cfg(feature = "rtos-trace")]
                    rtos_trace::trace::task_exec_begin(MAC_REQUEST);
                    self.handle_request(mac_request).await;
                    // TODO: Return a proper confirmation primitive.
                    self.request_receiver.received(response_token, ());
                }
                Either::Second(rx_frame) => {
                    #[cfg(feature = "rtos-trace")]
                    rtos_trace::trace::task_exec_begin(MAC_INDICATION);
                    self.handle_indication(rx_frame).await;
                }
            };
        }
    }

    /// Allocates a buffer for reception and then blocks until a frame was
    /// received and returns it. May be cancelled.
    async fn radio_recv(&self, rx_buffer_size: usize) -> RadioFrame<Config, RadioFrameSized> {
        // TODO: Currently we de-allocate and drop the buffer if reception is
        //       being cancelled. We may pre-allocate the radio frame instead
        //       and recover it on cancellation if it turns out that Rx buffer
        //       allocation becomes a performance bottleneck. We don't do this
        //       yet, as it would break encapsulation of this method even more
        //       than passing in a pre-calculated rx_buffer_size already does.
        let rx_buffer = self.buffer_allocator.allocate_buffer(rx_buffer_size).await;
        let rx_frame = DroppableRadioFrame::<Config, RadioFrameUnsized>::new(
            rx_buffer,
            self.buffer_allocator.allocator(),
        );
        let rx_radio_task = RadioTask::Rx(rx_frame);

        // Safety: To avoid deadlock, we always allocate channel capacity
        //         _after_ allocating a buffer.
        let rx_task_request_token = self.radio_task_sender.allocate_request_token().await;

        let rx_radio_task_response = self
            .radio_task_sender
            .send_request_and_wait(rx_task_request_token, rx_radio_task)
            .await;

        match rx_radio_task_response {
            RadioTaskResponse::Rx(rx_result) => match rx_result {
                Ok(rx_frame) => rx_frame,
                // Safety: An Rx radio task may only be cancelled when multiple
                //         consumers access the driver co-processor at at the
                //         same time. As this is not currently the case, we
                //         never expect an Err.
                Err(_) => unreachable!(),
            },
            // Safety: We scheduled an Rx task and therefore expect an Rx task
            //         response.
            _ => unreachable!(),
        }
    }

    /// Sends the given frame over the radio. Blocks until the frame was sent,
    /// then returns the frame for re-use.
    async fn radio_send(
        &self,
        tx_frame: RadioFrame<Config, RadioFrameSized>,
    ) -> RadioFrame<Config, RadioFrameSized> {
        let tx_radio_task = RadioTask::Tx(tx_frame);

        // Safety: To avoid deadlock, we always allocate channel capacity
        //         _after_ allocating a buffer. As the Tx frame is being passed
        //         in, we can be sure that the corresponding buffer has already
        //         been allocated.
        let tx_task_request_token = self.radio_task_sender.allocate_request_token().await;

        let tx_radio_task_response = self
            .radio_task_sender
            .send_request_and_wait(tx_task_request_token, tx_radio_task)
            .await;

        match tx_radio_task_response {
            RadioTaskResponse::Tx(tx_result) => match tx_result {
                Ok(tx_frame) => tx_frame,
                // TODO: Currently we assume that Tx always succeeds. Implement
                //       this branch depending on actual driver requirements.
                Err(_) => todo!("implement"),
            },
            // Safety: We scheduled a Tx task and therefore expect a Tx task
            //         response.
            _ => unreachable!(),
        }
    }

    async fn handle_indication(&self, rx_frame: RadioFrame<Config, RadioFrameSized>) {
        let rx_mpdu = MpduFrame::from_radio_frame(rx_frame);
        self.transmit_ack(&rx_mpdu).await;

        match rx_mpdu.frame_control().frame_type() {
            FrameType::Data => self.mcps_data_indication(rx_mpdu).await,
            FrameType::Beacon => self.mlme_beacon_notify_indication(rx_mpdu).await,
            _ => {}
        }
    }

    async fn handle_request(&self, request: MacRequest) {
        match request {
            MacRequest::McpsDataRequest(data_request) => {
                // TODO: handle errors with upper layer
                let _ = self.mcps_data_request(data_request).await;
            }
            MacRequest::MlmeBeaconRequest(beacon_request) => {
                // TODO: handle errors with upper layer
                let _ = self.mlme_beacon_request(&beacon_request).await;
            }
            MacRequest::MlmeSetRequest(set_request_attribute) => {
                // TODO: handle errors with upper layer
                let _ = self.mlme_set_request(&set_request_attribute).await;
            }
        }
    }
}
