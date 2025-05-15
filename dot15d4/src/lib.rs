#![no_std]

#[cfg(any(feature = "std", test))]
#[macro_use]
extern crate std;

#[macro_use]
pub(crate) mod utils;

use core::marker::PhantomData;

pub use dot15d4_frame3 as frame;
use dot15d4_frame3::driver::DriverConfig;
use mac::{MacBufferAllocator, MacIndicationSender, MacRequestReceiver};
use radio::RadioTaskChannel;

pub mod mac;
pub mod radio;
pub mod sync;
pub mod time;

use self::{mac::MacService, radio::DriverCoprocessor};
use crate::{
    radio::driver::RadioDriver,
    sync::{mutex::Mutex, select, Either},
};
use embedded_hal_async::delay::DelayNs;
use rand_core::RngCore;

pub struct Device<Config: DriverConfig, R: RadioDriver<Config>, Rng, TIMER> {
    radio: R,
    rng: Mutex<Rng>,
    timer: TIMER,
    driver_config: PhantomData<Config>,
}

impl<Config, R, Rng, TIMER> Device<Config, R, Rng, TIMER>
where
    Config: DriverConfig,
    R: RadioDriver<Config>,
    Rng: RngCore,
{
    pub fn new(radio: R, rng: Rng, timer: TIMER) -> Self {
        Self {
            radio,
            rng: Mutex::new(rng),
            timer,
            driver_config: PhantomData,
        }
    }
}

impl<Config: DriverConfig, R: RadioDriver<Config>, Rng: RngCore, TIMER: DelayNs + Clone>
    Device<Config, R, Rng, TIMER>
{
    pub async fn run<'upper_layer>(
        mut self,
        buffer_allocator: MacBufferAllocator,
        request_receiver: MacRequestReceiver<'upper_layer>,
        indication_sender: MacIndicationSender<'upper_layer>,
    ) -> ! {
        #[cfg(feature = "rtos-trace")]
        self::trace::instrument();

        let radio_task_channel = RadioTaskChannel::<Config>::new();
        let driver_coprocessor = DriverCoprocessor::new(self.radio, radio_task_channel.receiver());
        let mut mac_service = MacService::<'_, Rng, TIMER, Config>::new(
            &mut self.rng,
            self.timer.clone(),
            buffer_allocator,
            request_receiver,
            indication_sender,
            radio_task_channel.sender(),
        );

        match select::select(mac_service.run(), driver_coprocessor.run()).await {
            Either::First(_) => panic!("MAC service terminated"),
            Either::Second(_) => panic!("Driver coprocessor terminated"),
        }
    }

    // pub async fn start_as_coordinator(&mut self) {
    //     self.scan_energy().await;
    // }

    // pub async fn start(&mut self) {
    //     //
    //     self.scan_channels().await;
    // }

    // async fn receive_beacon_request<'a>(
    //     &self,
    //     buffer: &mut [u8; 128],
    //     radio_guard: &mut Option<MutexGuard<'a, R>>,
    // ) {
    //     receive(
    //         &mut **radio_guard.as_mut().unwrap(),
    //         buffer,
    //         RxConfig {
    //             channel: crate::phy::config::Channel::_26,
    //         },
    //     )
    //     .await;
    // }
}

#[cfg(feature = "rtos-trace")]
pub mod trace {
    #[cfg(feature = "defmt")]
    compile_error!(
        "Tracing cannot be enabled at the same time as defmt. Logs will be visible in the SystemView application if the 'log' feature is enabled."
    );

    pub const MAC_INDICATION: u32 = 0;
    pub const MAC_REQUEST: u32 = 1;
    pub const PHY_TX: u32 = 2;
    pub const PHY_RX: u32 = 3;

    /// Instrument the library for tracing.
    pub(super) fn instrument() {
        rtos_trace::trace::task_new_stackless(MAC_INDICATION, "MAC indication\0", 0);
        rtos_trace::trace::task_new_stackless(MAC_REQUEST, "MAC request\0", 0);
        rtos_trace::trace::task_new_stackless(PHY_TX, "PHY Tx\0", 0);
        rtos_trace::trace::task_new_stackless(PHY_RX, "PHY Rx\0", 0);
    }
}
