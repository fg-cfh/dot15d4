//! Access to IEEE 802.15.4 devices.
//!
//! This module provides access to IEEE 802.15.4 devices. It provides a trait
//! for transmitting and receiving frames, [Device].

pub mod config;
pub mod constants;
pub mod driver;
pub mod pib;

use core::{cell::RefCell, marker::PhantomData};

use dot15d4_frame3::driver::{
    DriverConfig, DroppableRadioFrame, RadioFrame, RadioFrameSized, RadioFrameUnsized,
};
use driver::RadioDriver;
use mpmc_channel::{AsyncChannel, AsyncSender, HasAddress, Receiver};

use crate::{select::select, sync::Either};

use self::config::{RxConfig, TxConfig};

/// Placeholder for future radio task abstraction.
pub enum RadioTask<Config: DriverConfig> {
    /// Frames to be sent on air must be sized, i.e. their PDU length must be
    /// defined.
    Tx(RadioFrame<Config, RadioFrameSized>),
    /// Frames to be filled by the driver with a PDU received on air must be
    /// empty, i.e. their PDU length cannot yet be known.
    Rx(DroppableRadioFrame<Config, RadioFrameUnsized>),
}

/// Currently the driver does not have an address. This may change when managing
/// several radios over the same channel.
impl<Config: DriverConfig> HasAddress<()> for RadioTask<Config> {
    fn address(&self) -> () {
        // no-op
    }
}

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum TxError {
    /// Ack failed, after too many retransmissions
    AckFailed,
    /// The buffer did not follow the correct device structure
    InvalidDeviceStructure,
    /// Something went wrong in the radio
    RadioError,
}

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum RxError {
    /// Rx was cancelled by another incoming radio task.
    Cancelled,
}

/// Placeholder for future radio task response abstraction.
pub enum RadioTaskResponse<Config: DriverConfig> {
    /// The response to a Tx driver task either communicates success or an error
    /// together with the unsent Tx frame.
    Tx(
        Result<
            // Sending successful: the sent frame is returned for re-use.
            RadioFrame<Config, RadioFrameSized>,
            // Sending not successful: an error is returned, together with the
            // sent frame for re-use.
            (TxError, RadioFrame<Config, RadioFrameSized>),
        >,
    ),
    /// The response to an Rx driver task either returns a received packet or an
    /// error together with the empty Rx frame.
    Rx(
        Result<
            // A frame was received: The received frame is returned.
            RadioFrame<Config, RadioFrameSized>,
            // A frame was not received: The empty frame is returned for re-use.
            (RxError, RadioFrame<Config, RadioFrameUnsized>),
        >,
    ),
}

// TODO: Make channel capacities configurable.
pub const DRIVER_CHANNEL_CAPACITY: usize = 2;
const DRIVER_CHANNEL_BACKLOG: usize = 0;

// TODO: Make the max driver overhead configurable.
pub const MAX_DRIVER_OVERHEAD: usize = 2;

pub type RadioTaskChannel<Config> = AsyncChannel<
    (),
    RadioTask<Config>,
    RadioTaskResponse<Config>,
    DRIVER_CHANNEL_CAPACITY,
    DRIVER_CHANNEL_BACKLOG,
    1,
>;
pub type RadioTaskReceiver<'channel, Config> = Receiver<
    'channel,
    (),
    RadioTask<Config>,
    RadioTaskResponse<Config>,
    DRIVER_CHANNEL_CAPACITY,
    1,
    RadioTaskChannel<Config>,
>;
pub type RadioTaskSender<'channel, Config> = AsyncSender<
    'channel,
    (),
    RadioTask<Config>,
    RadioTaskResponse<Config>,
    DRIVER_CHANNEL_CAPACITY,
    DRIVER_CHANNEL_BACKLOG,
    1,
>;

/// Structure managing the driver. Knows about and manages driver capabilities
/// and exposes a unified API to the MAC service.
pub struct DriverCoprocessor<'radio, Config: DriverConfig, R: RadioDriver<Config>> {
    // TODO: Remove the ref cell once the driver exposes an API on `&self`.
    radio: RefCell<R>,
    radio_task_receiver: RadioTaskReceiver<'radio, Config>,
    /// PAN Information Base
    pub pib: pib::Pib,
    driver_config: PhantomData<Config>,
}

impl<'radio, Config: DriverConfig, R: RadioDriver<Config>> DriverCoprocessor<'radio, Config, R> {
    /// Creates a new [`PhyService<Config, R>`].
    pub fn new(radio: R, radio_task_receiver: RadioTaskReceiver<'radio, Config>) -> Self {
        Self {
            radio: RefCell::new(radio),
            radio_task_receiver,
            pib: pib::Pib::default(),
            driver_config: PhantomData,
        }
    }

    /// Run the main event loop used by the PHY sublayer for its operation. For
    /// now, the loop waits for either receiving a frame from the MAC sublayer
    /// or receiving a frame from the radio.
    pub async fn run(&self) -> ! {
        let mut consumer_token = self
            .radio_task_receiver
            .try_allocate_consumer_token()
            .expect("no capacity");

        self.radio.borrow_mut().enable().await; // Wake up radio

        let (mut response_token, mut radio_task) = self
            .radio_task_receiver
            .wait_for_request(&mut consumer_token, &())
            .await;
        loop {
            (response_token, radio_task) = match radio_task {
                RadioTask::Tx(mut tx_frame) => {
                    #[cfg(feature = "rtos-trace")]
                    rtos_trace::trace::task_exec_begin(PHY_RX);
                    // Tx cannot be cancelled so we can unconditionally wait for a result.
                    let tx_result = self.tx_frame(&mut tx_frame).await;

                    // Send a tx result.
                    if tx_result {
                        // Success: The now unused frame will be sent back for re-use.
                        self.radio_task_receiver
                            .received(response_token, RadioTaskResponse::Tx(Ok(tx_frame)));
                    } else {
                        // Error: The current driver API does not give a reason
                        //        yet, so send back a generic radio error.
                        // TODO: Change the driver API to return an error code.
                        self.radio_task_receiver.received(
                            response_token,
                            RadioTaskResponse::Tx(Err((TxError::RadioError, tx_frame))),
                        );
                    }

                    // Await the next request.
                    self.radio_task_receiver
                        .wait_for_request(&mut consumer_token, &())
                        .await
                }
                RadioTask::Rx(rx_frame) => {
                    #[cfg(feature = "rtos-trace")]
                    rtos_trace::trace::task_exec_begin(PHY_TX);
                    let mut rx_frame = Some(rx_frame);
                    let rx_future = self.rx_frame(&mut rx_frame);
                    let next_task_future = self
                        .radio_task_receiver
                        .wait_for_request(&mut consumer_token, &());
                    match select(rx_future, next_task_future).await {
                        Either::First(rx_result) => {
                            // We received a driver frame, let's prepare and send an rx result.
                            self.radio_task_receiver
                                .received(response_token, RadioTaskResponse::Rx(Ok(rx_result)));

                            // Await the next request.
                            self.radio_task_receiver
                                .wait_for_request(&mut consumer_token, &())
                                .await
                        }
                        Either::Second((next_response_token, next_radio_task)) => {
                            // Let the sender know that the task was canceled.
                            // Note: This only happens when multiple producers
                            //       talk to the radio co-processor at the same
                            //       time. A single producer will usually cancel
                            //       ongoing reception which will drop the rx
                            //       frame (hence it is droppable). An
                            //       alternative would be for the radio
                            //       co-processor to receive a reference to a
                            //       "shared" radio frame that uses interior
                            //       mutability so that it can be recovered
                            //       after cancellation.
                            self.radio_task_receiver.received(
                                response_token,
                                RadioTaskResponse::Rx(Err((
                                    RxError::Cancelled,
                                    rx_frame.take().unwrap().into_non_droppable_frame(),
                                ))),
                            );

                            // We received a new radio task that canceled the previous one.
                            (next_response_token, next_radio_task)
                        }
                    }
                }
            }
        }
        //
    }

    /// Listen for a frame on the radio. The returned future may be cancelled.
    async fn rx_frame(
        &self,
        frame: &mut Option<DroppableRadioFrame<Config, RadioFrameUnsized>>,
    ) -> RadioFrame<Config, RadioFrameSized> {
        self.radio
            .borrow_mut()
            .receive(
                RxConfig {
                    channel: self.pib.current_channel.try_into().unwrap(),
                },
                frame,
            )
            .await
    }

    /// Transmit the given frame to the radio. Reports success.
    async fn tx_frame(&self, frame: &mut RadioFrame<Config, RadioFrameSized>) -> bool {
        self.radio
            .borrow_mut()
            .transmit(
                TxConfig {
                    channel: self.pib.current_channel.try_into().unwrap(),
                    ..Default::default()
                },
                frame,
            )
            .await
    }
}
