use core::future::Future;

use dot15d4_frame3::driver::{
    DriverConfig, DroppableRadioFrame, RadioFrame, RadioFrameSized, RadioFrameUnsized,
};

use super::config::{RxConfig, TxConfig};

pub trait RadioDriver<Config: DriverConfig> {
    /// Request the radio to idle to a low-power sleep mode.
    fn disable(&mut self) -> impl Future<Output = ()>;
    /// Request the radio to wake from sleep.
    fn enable(&mut self) -> impl Future<Output = ()>;

    /// Request the radio to go in receive mode and receive a frame into the
    /// supplied empty frame object. Always switches the radio to idle mode
    /// before it returns.
    ///
    /// When a buffer is received, then the driver will consume the given frame
    /// (leaving the option [`None`]) and return a sized driver frame with the
    /// received PDU.
    ///
    /// Canceling this future SHALL stop reception and leave the radio in the
    /// idle state. The given frame will then not be consumed and can either be
    /// re-used or dropped.
    fn receive(
        &mut self,
        cfg: RxConfig,
        frame: &mut Option<DroppableRadioFrame<Config, RadioFrameUnsized>>,
    ) -> impl Future<Output = RadioFrame<Config, RadioFrameSized>>;

    /// Request the radio to transmit the queued frame. Mutability proves that
    /// the buffer is in RAM to support DMA. Always switches the radio to idle
    /// mode before it returns.
    ///
    /// Returns whether transmission was successful.
    ///
    /// This future SHALL not be canceled.
    fn transmit(
        &mut self,
        cfg: TxConfig,
        frame: &mut RadioFrame<Config, RadioFrameSized>,
    ) -> impl Future<Output = bool>;

    /// Returns the IEEE802.15.4 8-octet MAC address of the radio device.
    fn ieee802154_address(&self) -> [u8; 8];
}

#[cfg(test)]
pub mod tests {
    use core::panic;
    use std::{
        collections::VecDeque,
        future::poll_fn,
        task::{Poll, Waker},
        vec::Vec,
    };

    use dot15d4_frame3::{
        driver::{DriverConfig, RadioFrame, Rx, Tx},
        FramePdu,
    };
    use embedded_hal_async::delay::DelayNs;
    use generic_array::GenericArray;

    use crate::{
        radio::config::{RxConfig, TxConfig},
        sync::{select, tests::StdDelay},
    };

    use super::RadioDriver;

    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum TestRadioEvent {
        Receive,
        Transmit,
        Disable,
        Enable,
    }

    pub struct TestRadio<Config: DriverConfig> {
        pub ieee802154_address: [u8; 8],
        pub should_receive: Option<generic_array::GenericArray<u8, Config::MaxFrameLen>>,
        pub events: Vec<TestRadioEvent>,
        pub cca_fail: bool,
        pub assert_nxt: VecDeque<TestRadioEvent>,
        pub total_event_count: usize,
        pub last_transmitted: Option<generic_array::GenericArray<u8, Config::MaxFrameLen>>,
        pub has_requested_cca: bool,
        assert_waker: Option<Waker>,
    }

    impl<Config: DriverConfig> TestRadio<Config> {
        pub fn new(ieee802154_address: [u8; 8]) -> Self {
            Self {
                ieee802154_address,
                should_receive: None,
                events: vec![],
                cca_fail: false,
                assert_nxt: VecDeque::new(),
                total_event_count: 0,
                last_transmitted: None,
                assert_waker: None,
                has_requested_cca: false,
            }
        }

        pub fn new_event(&mut self, event: TestRadioEvent) {
            // Filter duplicates
            if Some(&event) == self.events.last() {
                return;
            }

            println!(
                "New event arrived [{}]: {:?}",
                self.total_event_count, event
            );

            self.total_event_count += 1;
            if let Some(waker) = self.assert_waker.take() {
                waker.wake();
            }
            // Do not check if we are already panicking
            if std::thread::panicking() {
                return;
            }
            if let Some(assert_nxt) = self.assert_nxt.pop_front() {
                assert_eq!(
                    assert_nxt, event,
                    "Check if the next event is the expected event in the radio [{}](got {:?}, expected {:?})",
                    self.total_event_count, event, assert_nxt
                );
            }
            self.events.push(event);
        }

        /// Async wait for all radio events to have happened.
        /// This function is ment to be only used in tests an as such will panic
        /// if not all events have happened within 5s of starting
        pub async fn wait_until_asserts_are_consumed(&mut self) {
            let wait_for_events = poll_fn(|cx| {
                if self.assert_nxt.is_empty() {
                    Poll::Ready(())
                } else {
                    match &mut self.assert_waker {
                        Some(waker) if waker.will_wake(cx.waker()) => waker.clone_from(cx.waker()),
                        Some(waker) => {
                            waker.wake_by_ref();
                            waker.clone_from(cx.waker());
                        }
                        waker @ None => {
                            *waker = Some(cx.waker().clone());
                        }
                    };

                    Poll::Pending
                }
            });

            match select::select(wait_for_events, StdDelay::default().delay_ms(5000)).await {
                crate::sync::Either::First(_) => {}
                crate::sync::Either::Second(_) => {
                    if !std::thread::panicking() {
                        panic!("Waiting timed out for events -> there is a bug in the code")
                    }
                }
            }
        }
    }

    impl<Config: DriverConfig> Default for TestRadio<Config> {
        fn default() -> Self {
            Self::new([0xca; 8])
        }
    }

    impl<Config: DriverConfig> RadioDriver<Config> for TestRadio<Config> {
        async fn disable(&mut self) {
            self.new_event(TestRadioEvent::Disable);
        }

        async fn enable(&mut self) {
            self.new_event(TestRadioEvent::Enable);
        }

        async fn receive(
            &mut self,
            _cfg: RxConfig,
            frame: &mut RadioFrame<'_, Config, Rx>,
        ) -> Result<(), ()> {
            poll_fn(|cx| {
                cx.waker().wake_by_ref(); // Always wake immediatly again
                self.new_event(TestRadioEvent::Receive);

                if let Some(should_receive) = self.should_receive.take() {
                    frame.pdu_mut().copy_from_slice(should_receive.as_ref());

                    Poll::Ready(Ok(()))
                } else {
                    Poll::Pending
                }
            })
            .await
        }

        async fn transmit(
            &mut self,
            cfg: TxConfig,
            frame: &mut RadioFrame<'_, Config, Tx>,
        ) -> bool {
            self.new_event(TestRadioEvent::Transmit);
            let mut buffer = GenericArray::default();
            buffer.copy_from_slice(frame.pdu_ref());
            self.last_transmitted = Some(buffer);
            self.has_requested_cca = cfg.cca;
            !(self.has_requested_cca && self.cca_fail)
        }

        fn ieee802154_address(&self) -> [u8; 8] {
            self.ieee802154_address
        }
    }
}
