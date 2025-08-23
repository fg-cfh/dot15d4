//! nRF IEEE 802.15.4 radio driver

// TODO: In timed tx/rx to rx/tx we currently leave the transceiver enabled
//       after the previous packet has been received/transmitted and execute a
//       timed disable task. We then exploit the deterministic timing of
//       consecutive disable/enable shorts to ensure that the next task is
//       started at the right instant. This wastes energy between the end of the
//       previous packet and the timed disable task. It would be nicer if we
//       could leave the end-disable short active, race packet
//       reception/transmission with a timed disable task and once the CPU
//       catches the disabled event, start another timed re-enable task.
//       Unfortunately the disabled-to-reenable time is too short to be reliably
//       scheduled with our current timer design.
//
//       Alternatives:
//       - Add a second channel to the timer so that we can schedule both, the
//         disable and re-enable task concurrently. This is probably the most
//         stable and energy-efficient alternative as it will work w/o CPU
//         interaction between signals.
//       - Reduce the guard time of the timer, so that we can schedule signals
//         with lower latency (e.g. by adding a "leave high-precision-timer
//         running" flag or by using the RTC tick event to schedule low-latency
//         timeouts).

use core::{
    future::poll_fn,
    num::NonZero,
    sync::atomic::{compiler_fence, Ordering},
    task::Poll,
};

use dot15d4_util::{debug, frame::FramePdu, sync::CancellationGuard};
use nrf52840_hal::{
    clocks::{ExternalOscillator, LfOscStarted},
    pac::{self, radio::state::STATE_A},
    Clocks,
};
use typenum::U;

#[cfg(feature = "rtos-trace")]
use crate::radio::trace::{
    TASK_FALL_BACK, TASK_OFF_RUN, TASK_OFF_SCHEDULE, TASK_RX_FRAME_INFO, TASK_RX_FRAME_STARTED,
    TASK_RX_RUN, TASK_RX_SCHEDULE, TASK_TRANSITION_TO_OFF, TASK_TRANSITION_TO_RX,
    TASK_TRANSITION_TO_TX, TASK_TX_RUN, TASK_TX_SCHEDULE,
};
use crate::{
    config::{CcaMode, Channel},
    constants::{
        DEFAULT_SFD, FCS_LEN, MAC_AIFS, MAC_LIFS, MAC_SIFS, PHY_CCA_DURATION, PHY_HDR_LEN,
        PHY_MAX_PACKET_SIZE_127,
    },
    executor::InterruptExecutor,
    frame::{AddressingFields, RadioFrame, RadioFrameSized},
    radio::{DriverConfig, FcsNone, RadioDriver, RadioDriverApi},
    tasks::{
        ExternalRadioTransition, Ifs, OffResult, OffState, PreliminaryFrameInfo, RadioState,
        RadioTaskError, RadioTransition, RxError, RxResult, RxState, SchedulingError,
        SelfRadioTransition, TaskOff, TaskRx, TaskTx, Timestamp, TxError, TxResult, TxState,
    },
    timer::{
        HardwareSignal, RadioTimerApi, RadioTimerResult, SymbolsOQpsk250Duration,
        SyntonizedDuration, TimedSignal,
    },
};

use super::{executor::radio::NrfInterruptExecutor, timer::NrfRadioTimer};

pub mod export {
    pub use nrf52840_hal::{
        clocks::{Clocks, ExternalOscillator, LfOscConfiguration, LfOscStarted},
        pac,
        rng::Rng,
    };
}

// The nRF hardware only supports default CCA duration.
const _: () = assert!(PHY_CCA_DURATION.ticks() == 8);

// Disabled to Tx Idle duration
const T_TXEN: SyntonizedDuration = SyntonizedDuration::micros(130);
// Tx Idle to Disabled duration
const T_TXDIS: SyntonizedDuration = SyntonizedDuration::micros(21);
// Disabled to Rx Idle duration
const T_RXEN: SyntonizedDuration = SyntonizedDuration::micros(130);
// Rx Idle to Disabled duration
const T_RXDIS: SyntonizedDuration = SyntonizedDuration::nanos(500);
// CCA duration
const T_CCA: SyntonizedDuration = PHY_CCA_DURATION.convert();
// Rx-to-Tx and Tx-to-Rx duration
const T_TURNAROUND: SyntonizedDuration = SyntonizedDuration::micros(40);
// SHR duration: preamble (8 symbols) + SFD (2 symbols)
const T_SHR: SyntonizedDuration = SymbolsOQpsk250Duration::from_ticks(10).convert();

/// This struct serves multiple purposes:
/// 1. It provides access to private radio driver state across typestates of the
///    surrounding [`RadioDriver`].
/// 2. It serves as a unique marker for the nRF-specific implementation of the
///    [`RadioDriver`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NrfRadioDriver {
    executor: NrfInterruptExecutor,
}

impl DriverConfig for NrfRadioDriver {
    type Headroom = U<PHY_HDR_LEN>; // Headroom for the PHY header (packet length).
    type Tailroom = U<FCS_LEN>; // Tailroom for driver-level FCS handling.
    type MaxSduLength = U<{ PHY_MAX_PACKET_SIZE_127 - FCS_LEN }>; // The FCS is handled by the driver and must not be part of the MAC's MPDU.
    type Fcs = FcsNone; // Assuming automatic FCS handling.
    type Timer = NrfRadioTimer;
}

impl NrfRadioDriver {
    fn radio() -> pac::RADIO {
        // Safety: We let clients prove unique ownership of the peripheral by
        //         requiring an instance when instantiating the driver.
        unsafe { pac::Peripherals::steal() }.RADIO
    }
}

impl<Task> RadioDriver<NrfRadioDriver, Task> {
    /// Convenience shortcut to access the radio registers.
    ///
    /// Safety: see [`NrfRadioDriver::radio()`]
    fn radio() -> pac::RADIO {
        NrfRadioDriver::radio()
    }

    fn set_ifs(ifs: Ifs) {
        const AIFS_US: u16 = MAC_AIFS.to_micros() as u16;
        const SIFS_US: u16 = MAC_SIFS.to_micros() as u16;
        const LIFS_US: u16 = MAC_LIFS.to_micros() as u16;

        let tifs_us = match ifs {
            Ifs::Aifs => AIFS_US,
            Ifs::Sifs => SIFS_US,
            Ifs::Lifs => LIFS_US,
            Ifs::None => 0,
        };

        Self::radio().tifs.write(|w| w.tifs().variant(tifs_us));
    }

    const fn timed_off(off_task: &TaskOff) -> Option<TimedSignal> {
        if let Timestamp::Scheduled(off_timestamp) = off_task.at {
            Some(TimedSignal::new(
                off_timestamp,
                HardwareSignal::RadioDisable,
            ))
        } else {
            None
        }
    }
}

/// RX bit counter event triggered after the frame control field (2 bytes) has
/// been received.
const BCC_FC_BITS: u32 = 2 * 8;

/// Radio Off state.
///
/// Entry: DISABLED event
/// Exit: READY event
///
/// State Invariants:
/// - The radio is in the DISABLED state.
/// - The READY event has been cleared.
/// - Only the READY interrupt is enabled.
///
/// The disabled state remains stable unless some task is actively
/// triggered.
impl RadioDriver<NrfRadioDriver, TaskOff> {
    /// Create a new IEEE 802.15.4 radio driver.
    ///
    /// Safety:
    /// - The constructor ensures that clients transfer exclusive ownership of
    ///   the radio peripheral. This also ensures that only a single instance of
    ///   the driver can be created in safe code.
    /// - The constructor lets clients prove proper configuration of the clocks
    ///   peripheral.
    pub fn new(
        radio: pac::RADIO,
        _clocks: Clocks<ExternalOscillator, ExternalOscillator, LfOscStarted>,
        timer: NrfRadioTimer,
    ) -> Self {
        #[cfg(feature = "rtos-trace")]
        crate::radio::trace::instrument();

        // Disable and enable to reset peripheral
        radio.power.write(|w| w.power().disabled());
        radio.power.write(|w| w.power().enabled());

        // Enable 802.15.4 mode
        radio.mode.write(|w| w.mode().ieee802154_250kbit());
        // Configure CRC skip address
        radio
            .crccnf
            .write(|w| w.len().two().skipaddr().ieee802154());
        // Configure CRC polynomial and init
        radio.crcpoly.write(|w| w.crcpoly().variant(0x0001_1021));
        radio.crcinit.write(|w| w.crcinit().variant(0));
        radio.pcnf0.write(|w| {
            // 8-bit on air length
            w.lflen().variant(8);
            // Zero bytes S0 field length
            w.s0len().clear_bit();
            // Zero bytes S1 field length
            w.s1len().variant(0);
            // Do not include S1 field in RAM if S1 length > 0
            w.s1incl().automatic();
            // Zero code Indicator length
            w.cilen().variant(0);
            // 32-bit zero preamble
            w.plen()._32bit_zero();
            // Include CRC in length
            w.crcinc().include()
        });
        radio.pcnf1.write(|w| {
            // Maximum packet length
            w.maxlen().variant(PHY_MAX_PACKET_SIZE_127 as u8);
            // Zero static length
            w.statlen().variant(0);
            // Zero base address length
            w.balen().variant(0);
            // Little-endian
            w.endian().little();
            // Disable packet whitening
            w.whiteen().clear_bit()
        });
        // Default ramp-up mode for TIFS support.
        radio.modecnf0.write(|w| w.ru().default());

        // Configure the RX bit counter to trigger once the frame control field
        // has been received.
        radio.bcc.write(|w| w.bcc().variant(BCC_FC_BITS));

        let mut driver = Self {
            inner: NrfRadioDriver {
                executor: *super::executor::radio(radio),
            },
            task: Some(TaskOff {
                at: Timestamp::BestEffort,
            }),
            timer,
        };

        driver.set_sfd(DEFAULT_SFD);
        driver.set_tx_power(0);
        driver.set_channel(Channel::_11);
        driver.set_cca(CcaMode::CarrierSense);

        driver
    }

    /// Changes the Clear Channel Assessment method
    pub fn set_cca(&mut self, cca: CcaMode) {
        let r = Self::radio();
        match cca {
            CcaMode::CarrierSense => r.ccactrl.write(|w| w.ccamode().carrier_mode()),
            CcaMode::EnergyDetection { ed_threshold } => {
                // "[ED] is enabled by first configuring the field CCAMODE=EdMode in CCACTRL
                // and writing the CCAEDTHRES field to a chosen value."
                r.ccactrl.write(|w| {
                    w.ccamode().ed_mode();
                    w.ccaedthres().variant(ed_threshold)
                });
            }
        }
    }

    /// Changes the Start of Frame Delimiter (SFD)
    pub fn set_sfd(&mut self, sfd: u8) {
        Self::radio().sfd.write(|w| w.sfd().variant(sfd));
    }

    /// Changes the radio transmission power
    pub fn set_tx_power(&mut self, power: i8) {
        Self::radio().txpower.write(|w| match power {
            #[cfg(not(any(feature = "nrf52811", feature = "nrf5340-net")))]
            8 => w.txpower().pos8d_bm(),
            #[cfg(not(any(feature = "nrf52811", feature = "nrf5340-net")))]
            7 => w.txpower().pos7d_bm(),
            #[cfg(not(any(feature = "nrf52811", feature = "nrf5340-net")))]
            6 => w.txpower().pos6d_bm(),
            #[cfg(not(any(feature = "nrf52811", feature = "nrf5340-net")))]
            5 => w.txpower().pos5d_bm(),
            #[cfg(not(feature = "nrf5340-net"))]
            4 => w.txpower().pos4d_bm(),
            #[cfg(not(feature = "nrf5340-net"))]
            3 => w.txpower().pos3d_bm(),
            #[cfg(not(any(feature = "nrf52811", feature = "nrf5340-net")))]
            2 => w.txpower().pos2d_bm(),
            0 => w.txpower()._0d_bm(),
            #[cfg(feature = "nrf5340-net")]
            -1 => w.txpower().neg1d_bm(),
            #[cfg(feature = "nrf5340-net")]
            -2 => w.txpower().neg2d_bm(),
            #[cfg(feature = "nrf5340-net")]
            -3 => w.txpower().neg3d_bm(),
            -4 => w.txpower().neg4d_bm(),
            #[cfg(feature = "nrf5340-net")]
            -5 => w.txpower().neg5d_bm(),
            #[cfg(feature = "nrf5340-net")]
            -6 => w.txpower().neg6d_bm(),
            #[cfg(feature = "nrf5340-net")]
            -7 => w.txpower().neg7d_bm(),
            -8 => w.txpower().neg8d_bm(),
            -12 => w.txpower().neg12d_bm(),
            -16 => w.txpower().neg16d_bm(),
            -20 => w.txpower().neg20d_bm(),
            -40 => w.txpower().neg40d_bm(),
            _ => panic!("Invalid transmission power value"),
        });
    }

    const fn timed_dis_to_rx(rx_task: &TaskRx) -> Option<TimedSignal> {
        if let Timestamp::Scheduled(rx_timestamp) = rx_task.start {
            // RMARKER offset: Disabled -> Rx -> SHR
            const OFFSET: SyntonizedDuration = T_RXEN.checked_add(T_SHR).unwrap();
            Some(TimedSignal::new(
                rx_timestamp.checked_sub_duration(OFFSET).unwrap(),
                HardwareSignal::RadioRxEnable,
            ))
        } else {
            None
        }
    }

    const fn timed_dis_to_tx(tx_task: &TaskTx) -> Option<TimedSignal> {
        if let Timestamp::Scheduled(tx_timestamp) = tx_task.at {
            let timed_signal = if tx_task.cca {
                // RMARKER offset with CCA: Disabled -> Rx -> CCA -> Turnaround -> SHR
                const OFFSET_DIS_TO_TX_W_CCA: SyntonizedDuration = T_RXEN
                    .checked_add(T_CCA)
                    .unwrap()
                    .checked_add(T_TURNAROUND)
                    .unwrap()
                    .checked_add(T_SHR)
                    .unwrap();
                TimedSignal::new(
                    tx_timestamp
                        .checked_sub_duration(OFFSET_DIS_TO_TX_W_CCA)
                        .unwrap(),
                    HardwareSignal::RadioRxEnable,
                )
            } else {
                // RMARKER offset without CCA: Disabled -> Tx -> SHR
                const OFFSET_DIS_TO_TX_NO_CCA: SyntonizedDuration =
                    T_TXEN.checked_add(T_SHR).unwrap();
                TimedSignal::new(
                    tx_timestamp
                        .checked_sub_duration(OFFSET_DIS_TO_TX_NO_CCA)
                        .unwrap(),
                    HardwareSignal::RadioTxEnable,
                )
            };
            Some(timed_signal)
        } else {
            None
        }
    }
}

impl<State> RadioDriverApi for RadioDriver<NrfRadioDriver, State> {
    fn ieee802154_address(&self) -> [u8; 8] {
        // Safety: Read-only access to a read-only register.
        let ficr: nrf52840_hal::pac::FICR = unsafe { pac::Peripherals::steal() }.FICR;
        let id1 = ficr.deviceid[0].read().bits(); // TODO: Should this be modified to use DEVICEADDR (only 48bit)?
        let id2 = ficr.deviceid[1].read().bits();
        [
            ((id1 & 0xff000000u32) >> 24u32) as u8,
            ((id1 & 0x00ff0000u32) >> 16u32) as u8,
            ((id1 & 0x0000ff00u32) >> 8u32) as u8,
            (id1 & 0x000000ffu32) as u8,
            ((id2 & 0xff000000u32) >> 24u32) as u8,
            ((id2 & 0x00ff0000u32) >> 16u32) as u8,
            ((id2 & 0x0000ff00u32) >> 8u32) as u8,
            (id2 & 0x000000ffu32) as u8,
        ]
    }
}

impl RadioState<TaskOff> for RadioDriver<NrfRadioDriver, TaskOff> {
    async fn transition(&mut self) -> Result<(), RadioTaskError<TaskOff>> {
        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::task_exec_begin(TASK_TRANSITION_TO_OFF);

        // Wait until the state enters.
        unsafe {
            self.inner
                .executor
                .spawn(poll_fn(|_| {
                    let r = Self::radio();
                    if r.events_disabled.read().events_disabled().bit_is_set() {
                        r.intenclr.write(|w| w.disabled().set_bit());
                        r.events_disabled.reset();
                        Poll::Ready(())
                    } else {
                        r.intenset.write(|w| w.disabled().set_bit());
                        Poll::Pending
                    }
                }))
                .await;
        }

        Ok(())
    }

    async fn run(
        &mut self,
        timed_transition: Option<TimedSignal>,
        _: bool,
    ) -> Result<OffResult, RadioTaskError<TaskOff>> {
        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::task_exec_begin(TASK_OFF_RUN);

        if let Some(timed_transition) = timed_transition {
            let result = unsafe {
                self.timer()
                    .wait_until(timed_transition.instant, Some(timed_transition.signal))
                    .await
            };
            if matches!(result, RadioTimerResult::Overdue) {
                return Err(RadioTaskError::Scheduling(SchedulingError));
            }
        }

        Ok(OffResult::Off)
    }

    fn exit(&mut self) -> Result<(), SchedulingError> {
        Ok(())
    }
}

impl OffState<NrfRadioDriver> for RadioDriver<NrfRadioDriver, TaskOff> {
    /// Changes the default radio channel
    fn set_channel(&mut self, channel: Channel) {
        let channel: u8 = channel.into();
        let frequency_offset = (channel - 10) * 5;
        Self::radio()
            .frequency
            .write(|w| w.frequency().variant(frequency_offset).map().default());
    }

    fn schedule_rx(
        self,
        rx_task: TaskRx,
    ) -> impl ExternalRadioTransition<NrfRadioDriver, TaskOff, TaskRx> {
        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::task_exec_begin(TASK_RX_SCHEDULE);

        let packetptr = rx_task.radio_frame.as_ptr() as u32;
        dma_start_fence();

        let timed_rxen = Self::timed_dis_to_rx(&rx_task);
        RadioTransition::new(
            self,
            rx_task,
            timed_rxen,
            move || {
                let r = Self::radio();

                // Ramp up the receiver and start packet reception immediately.
                r.packetptr.write(|w| w.packetptr().variant(packetptr));

                r.shorts.write(|w| {
                    w.rxready_start().enabled();
                    w.framestart_bcstart().enabled()
                });

                if timed_rxen.is_none() {
                    r.tasks_rxen.write(|w| w.tasks_rxen().set_bit());
                }

                Ok(())
            },
            || Ok(()),
            || {
                Self::radio()
                    .shorts
                    .modify(|_, w| w.rxready_start().disabled());
                Ok(())
            },
            false,
        )
    }

    fn schedule_tx(
        self,
        mut tx_task: TaskTx,
    ) -> impl ExternalRadioTransition<NrfRadioDriver, TaskOff, TaskTx> {
        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::task_exec_begin(TASK_TX_SCHEDULE);

        let packetptr = prepare_tx_frame(&mut tx_task.radio_frame);
        dma_start_fence();

        let timed_txen = Self::timed_dis_to_tx(&tx_task);
        let cca = tx_task.cca;
        RadioTransition::new(
            self,
            tx_task,
            timed_txen,
            move || {
                let r = Self::radio();

                r.packetptr.write(|w| w.packetptr().variant(packetptr));
                if cca {
                    r.shorts.write(|w| {
                        // Start CCA immediately after the receiver ramped up.
                        w.rxready_ccastart().enabled();
                        // If the channel is idle, then ramp up the transmitter and start TX
                        // immediately.
                        w.ccaidle_txen().enabled();
                        w.txready_start().enabled();
                        // If the channel is busy, then disable the receiver.
                        w.ccabusy_disable().enabled()
                    });

                    if timed_txen.is_none() {
                        r.tasks_rxen.write(|w| w.tasks_rxen().set_bit());
                    }
                } else {
                    r.shorts.write(|w| w.txready_start().enabled());
                    if timed_txen.is_none() {
                        r.tasks_txen.write(|w| w.tasks_txen().set_bit());
                    }
                }

                Ok(())
            },
            || Ok(()),
            || {
                // Cleanup shorts.
                Self::radio().shorts.reset();
                Ok(())
            },
            false,
        )
    }

    async fn switch_off<AnyState>(any_state: RadioDriver<NrfRadioDriver, AnyState>) -> Self {
        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::task_exec_begin(TASK_FALL_BACK);

        let off_task = Some(TaskOff {
            at: Timestamp::BestEffort,
        });
        let RadioDriver { inner, timer, .. } = any_state;
        let mut off_state = Self {
            inner,
            timer,
            task: off_task,
        };

        let r = Self::radio();
        match r.state.read().state().variant().unwrap() {
            STATE_A::DISABLED => return off_state,
            STATE_A::RX_RU
            | STATE_A::RX_IDLE
            | STATE_A::RX
            | STATE_A::TX_RU
            | STATE_A::TX_IDLE
            | STATE_A::TX => {
                r.tasks_disable.write(|w| w.tasks_disable().set_bit());
            }
            STATE_A::TX_DISABLE | STATE_A::RX_DISABLE => {}
        }

        let is_off = off_state.transition().await;
        debug_assert!(is_off.is_ok());

        off_state
    }
}

/// Radio reception state.
///
/// Entry: READY event (coming from a non-RX state), otherwise RX state
/// Exit: END event
///
/// State Invariants:
/// - The radio is in the RX or RXIDLE state.
/// - The radio's DMA pointer points to an empty, writable packet in RAM.
/// - The "END" and "CRCERROR" events have been cleared before starting reception.
/// - Only the "END" interrupt is enabled.
impl RadioDriver<NrfRadioDriver, TaskRx> {
    const fn timed_rx_to_tx(tx_task: &TaskTx) -> Option<TimedSignal> {
        if let Timestamp::Scheduled(tx_timestamp) = tx_task.at {
            let timed_signal = if tx_task.cca {
                // RMARKER offset with CCA: CCA -> Turnaround -> SHR
                const OFFSET_RX_TO_TX_W_CCA: SyntonizedDuration = T_CCA
                    .checked_add(T_TURNAROUND)
                    .unwrap()
                    .checked_add(T_SHR)
                    .unwrap();
                // TODO: We assume that we can start CCA even during ongoing
                //       reception (which should result in CCA busy). This needs
                //       to be tested.
                TimedSignal::new(
                    tx_timestamp
                        .checked_sub_duration(OFFSET_RX_TO_TX_W_CCA)
                        .unwrap(),
                    HardwareSignal::RadioCCA,
                )
            } else {
                // RMARKER offset without CCA: Rx -> Disabled -> Tx -> SHR
                const OFFSET_RX_TO_TX_NO_CCA: SyntonizedDuration = T_RXDIS
                    .checked_add(T_TXEN)
                    .unwrap()
                    .checked_add(T_SHR)
                    .unwrap();
                TimedSignal::new(
                    tx_timestamp
                        .checked_sub_duration(OFFSET_RX_TO_TX_NO_CCA)
                        .unwrap(),
                    HardwareSignal::RadioDisable,
                )
            };
            Some(timed_signal)
        } else {
            None
        }
    }
}

impl RadioState<TaskRx> for RadioDriver<NrfRadioDriver, TaskRx> {
    async fn transition(&mut self) -> Result<(), RadioTaskError<TaskRx>> {
        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::task_exec_begin(TASK_TRANSITION_TO_RX);

        // Wait until the state enters.
        unsafe {
            self.inner
                .executor
                .spawn(poll_fn(|_| {
                    let r = Self::radio();
                    if r.events_rxready.read().events_rxready().bit_is_set() {
                        r.intenclr.write(|w| w.rxready().set_bit());
                        r.events_rxready.reset();
                        Poll::Ready(())
                    } else {
                        // Double check state in case we're coming from another RX state
                        // (CCA).
                        match r.state.read().state().variant() {
                            Some(STATE_A::RX | STATE_A::RX_IDLE) => Poll::Ready(()),
                            _ => {
                                r.intenset.write(|w| w.rxready().set_bit());
                                Poll::Pending
                            }
                        }
                    }
                }))
                .await;
        }

        Ok(())
    }

    async fn run(
        &mut self,
        timed_transition: Option<TimedSignal>,
        rollback_on_crcerror: bool,
    ) -> Result<RxResult, RadioTaskError<TaskRx>> {
        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::task_exec_begin(TASK_RX_RUN);

        let r = Self::radio();

        // Wait until the task completed.
        let is_back_to_back_rx = r.shorts.read().end_start().is_enabled();
        if let Some(timed_transition) = timed_transition {
            let result = unsafe {
                self.timer()
                    .wait_until(timed_transition.instant, Some(timed_transition.signal))
                    .await
            };
            if matches!(result, RadioTimerResult::Overdue) {
                return Err(RadioTaskError::Scheduling(SchedulingError));
            }
        } else {
            // Read the framestart event at the last possible moment to minimize the
            // risk of missing a frame.
            let reception_may_be_ongoing =
                r.events_framestart.read().events_framestart().bit_is_set();
            if reception_may_be_ongoing || is_back_to_back_rx {
                // Ongoing best-effort reception or RX back-to-back.

                // Wait until the remainder of the packet has been received and the
                // receiver becomes idle.
                unsafe {
                    self.inner
                        .executor
                        .spawn(poll_fn(|_| {
                            if r.events_end.read().events_end().bit_is_set() {
                                r.intenclr.write(|w| w.end().set_bit());
                                Poll::Ready(())
                            } else {
                                r.intenset.write(|w| w.end().set_bit());
                                Poll::Pending
                            }
                        }))
                        .await;
                }
            } else {
                // Actively cancel the ongoing task and disable the receiver.
                r.tasks_disable.write(|w| w.tasks_disable().set_bit());
            }
        }

        dma_end_fence();

        // Clear the framestart flag _after_ the receiver became idle to avoid
        // race conditions.
        r.events_framestart.reset();
        // We reset the BCMATCH event here just in case we didn't
        // retrieve the preliminary frame info for the last RX
        // packet (e.g. if it was an ACK packet) and therefore also
        // didn't reset the event.
        r.events_bcmatch.reset();
        r.events_end.reset();

        // We're now either in RX idle state or transitioning towards disabled state.
        if r.events_crcok.read().events_crcok().bit_is_set() {
            r.events_crcok.reset();

            // The CRC has been checked so the frame must have a non-zero
            // size saved in the headroom of the nRF packet (PHY header).
            let rx_task = self.task.take().unwrap();
            let sdu_length_wo_fcs =
                NonZero::new(rx_task.radio_frame.pdu_ref()[0] as u16 - FCS_LEN as u16)
                    .expect("invalid length");

            Ok(RxResult::Frame(
                rx_task.radio_frame.with_size(sdu_length_wo_fcs),
            ))
        } else if r.events_crcerror.read().events_crcerror().bit_is_set() {
            r.events_crcerror.reset();

            if rollback_on_crcerror {
                // When rolling back we're expected to place the radio in RX
                // mode again and receive the next packet into the same
                // buffer. Therefore restart the receiver unless it was
                // already started. Not required for back-to-back Rx as the
                // radio will be re-started by a short in that case.
                if !is_back_to_back_rx {
                    r.tasks_start.write(|w| w.tasks_start().set_bit());
                }
                Err(RadioTaskError::Task(RxError::CrcError))
            } else {
                let rx_task = self.task.take().unwrap();
                Ok(RxResult::CrcError(rx_task.radio_frame))
            }
        } else {
            let rx_task = self.task.take().unwrap();
            Ok(RxResult::RxWindowEnded(rx_task.radio_frame))
        }
    }

    fn exit(&mut self) -> Result<(), SchedulingError> {
        Self::radio()
            .shorts
            .modify(|_, w| w.framestart_bcstart().disabled());
        Ok(())
    }
}

impl RxState<NrfRadioDriver> for RadioDriver<NrfRadioDriver, TaskRx> {
    async fn frame_started(&mut self) {
        unsafe {
            self.inner
                .executor
                .spawn(async {
                    let r = Self::radio();

                    let cleanup_on_drop = CancellationGuard::new(|| {
                        r.intenclr.write(|w| w.framestart().set_bit());
                    });

                    poll_fn(|_| {
                        if r.events_framestart.read().events_framestart().bit_is_set() {
                            // Do not clear the framestart event as it is used in the RX run()
                            // method.
                            r.intenclr.write(|w| w.framestart().set_bit());
                            Poll::Ready(())
                        } else {
                            r.intenset.write(|w| w.framestart().set_bit());
                            Poll::Pending
                        }
                    })
                    .await;

                    cleanup_on_drop.inactivate();
                })
                .await;
        }

        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::marker(TASK_RX_FRAME_STARTED);
    }

    async fn preliminary_frame_info(&mut self) -> PreliminaryFrameInfo<'_> {
        // Wait until the frame control field has been received.
        const FC_LEN: usize = 2;
        const SEQ_NR_LEN: usize = 1;

        let mut preliminary_frame_info: Option<PreliminaryFrameInfo> = None;

        let capture_preliminary_frame_info = async {
            let r = Self::radio();

            let cleanup_on_drop = CancellationGuard::new(|| {
                r.intenclr.write(|w| {
                    w.bcmatch().set_bit();
                    w.end().set_bit()
                });
                // Do not clear the end event as it is used in the RX run()
                // method.
                r.tasks_bcstop.write(|w| w.tasks_bcstop().set_bit());
                r.bcc.write(|w| w.bcc().variant(BCC_FC_BITS));
                r.events_bcmatch.reset();
            });

            let ended = poll_fn(|_| {
                let bcmatch = r.events_bcmatch.read().events_bcmatch().bit_is_set();
                if bcmatch || r.events_end.read().events_end().bit_is_set() {
                    // Do not clear the end event as it is used below and in
                    // the RX run() method.
                    r.intenclr.write(|w| {
                        w.bcmatch().set_bit();
                        w.end().set_bit()
                    });
                    r.events_bcmatch.reset();
                    return Poll::Ready(!bcmatch);
                }

                r.intenset.write(|w| {
                    w.bcmatch().set_bit();
                    w.end().set_bit()
                });
                Poll::Pending
            })
            .await;

            dma_end_fence();

            if ended && r.events_crcerror.read().events_crcerror().bit_is_set() {
                return;
            }

            let radio_frame = &self.task.as_ref().unwrap().radio_frame;

            // Safety: The bit counter match guarantees that the frame
            //         control field has been received.
            let fc_and_addressing_repr = unsafe { radio_frame.fc_and_addressing_repr() };

            if fc_and_addressing_repr.is_err() {
                return;
            }

            let (frame_control, addressing_repr) = fc_and_addressing_repr.unwrap();
            let addressing_fields_lengths = addressing_repr.addressing_fields_lengths();

            if addressing_fields_lengths.is_err() {
                return;
            }

            let seq_nr_len = if frame_control.sequence_number_suppression() {
                0
            } else {
                SEQ_NR_LEN
            };

            let [dst_pan_id_len, dst_addr_len, ..] = addressing_fields_lengths.unwrap();
            let dst_len = (dst_pan_id_len + dst_addr_len) as usize;

            let pdu_ref = radio_frame.pdu_ref();
            let mpdu_length = pdu_ref[0] as u16;

            if seq_nr_len == 0 && dst_len == 0 {
                preliminary_frame_info = Some(PreliminaryFrameInfo {
                    mpdu_length,
                    frame_control: Some(frame_control),
                    seq_nr: None,
                    addressing_fields: None,
                });
                return;
            }

            let ended = if !ended {
                // Note: BCMATCH counts are calculated relative to the MPDU
                //       (i.e. w/o headroom).
                let bcc = ((FC_LEN + seq_nr_len + dst_len) as u32) << 3;

                // Wait until the sequence number and/or destination address
                // fields have been received.
                r.bcc.write(|w| w.bcc().variant(bcc));

                poll_fn(|_| {
                    let bcmatch = r.events_bcmatch.read().events_bcmatch().bit_is_set();
                    if bcmatch || r.events_end.read().events_end().bit_is_set() {
                        // Do not clear the end event as it is used in the RX run()
                        // method. The remaining cleanup will be done by the
                        // cancel guard.
                        return Poll::Ready(!bcmatch);
                    }

                    r.intenset.write(|w| {
                        w.bcmatch().set_bit();
                        w.end().set_bit()
                    });
                    Poll::Pending
                })
                .await
            } else {
                true
            };

            drop(cleanup_on_drop);

            dma_end_fence();

            if ended {
                if r.events_crcerror.read().events_crcerror().bit_is_set() {
                    return;
                } else {
                    debug_assert!(r.events_crcok.read().events_crcok().bit_is_set());
                }
            }

            const HEADROOM: usize = 1;

            let seq_nr_offset = HEADROOM + FC_LEN;
            let seq_nr = if frame_control.sequence_number_suppression() {
                None
            } else {
                Some(pdu_ref[seq_nr_offset])
            };

            let addressing_offset = seq_nr_offset + seq_nr_len;
            let addressing_bytes = &pdu_ref[addressing_offset..];

            // Safety: The bit counter guarantees that all bytes up to the
            //         addressing fields have been received.
            let addressing_fields =
                unsafe { AddressingFields::new_unchecked(addressing_bytes, addressing_repr).ok() };

            preliminary_frame_info = Some(PreliminaryFrameInfo {
                mpdu_length,
                frame_control: Some(frame_control),
                seq_nr,
                addressing_fields,
            });
        };
        unsafe {
            self.inner
                .executor
                .spawn(capture_preliminary_frame_info)
                .await
        };

        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::marker(TASK_RX_FRAME_INFO);

        preliminary_frame_info.unwrap_or(PreliminaryFrameInfo {
            mpdu_length: 0,
            frame_control: None,
            seq_nr: None,
            addressing_fields: None,
        })
    }

    fn schedule_rx(
        self,
        rx_task: TaskRx,
        rollback_on_crcerror: bool,
    ) -> impl SelfRadioTransition<NrfRadioDriver, TaskRx, TaskRx> {
        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::task_exec_begin(TASK_RX_SCHEDULE);

        if let Timestamp::Scheduled(_) = rx_task.start {
            // See the note re back-to-back scheduling in the API.
            panic!("not supported")
        }

        let packetptr = rx_task.radio_frame.as_ptr() as u32;
        dma_start_fence();

        RadioTransition::new(
            self,
            rx_task,
            None,
            move || {
                let r = Self::radio();

                r.packetptr.write(|w| w.packetptr().variant(packetptr));

                // Enable back-to-back packet reception.
                //
                // NOTE: We need to set up the short before checking radio state to
                //       avoid race conditions, see RX_IDLE case below.
                r.shorts.write(|w| {
                    w.end_start().enabled();
                    w.framestart_bcstart().enabled()
                });

                // Check whether the task has already completed.
                //
                // NOTE: Read the state _after_ having set the short.
                match r.state.read().state().variant() {
                    Some(STATE_A::RX_IDLE) => {
                        // We're idle, although we have a short in place: This means
                        // that the previous packet was fully received before we were
                        // able to set the short, i.e. reception of the new packet was
                        // not started by hardware, we need to start it manually, see
                        // conditions 1. and 2. in the method documentation.
                        r.tasks_start.write(|w| w.tasks_start().set_bit());

                        debug!("late scheduling");
                    }
                    Some(STATE_A::RX) => {
                        // We're still receiving the previous packet (i.e., END pending,
                        // condition 3.).
                    }
                    _ => unreachable!(),
                };

                Ok(())
            },
            || Ok(()),
            || {
                Self::radio().shorts.modify(|_, w| w.end_start().disabled());
                Ok(())
            },
            rollback_on_crcerror,
        )
    }

    fn schedule_tx(
        self,
        mut tx_task: TaskTx,
        ifs: Ifs,
        rollback_on_crcerror: bool,
    ) -> impl ExternalRadioTransition<NrfRadioDriver, TaskRx, TaskTx> {
        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::task_exec_begin(TASK_TX_SCHEDULE);

        // PACKETPTR is double buffered so we don't cause a race by setting it
        // while reception might still be ongoing.
        let packetptr = prepare_tx_frame(&mut tx_task.radio_frame);
        dma_start_fence();

        let timed_txen = Self::timed_rx_to_tx(&tx_task);
        let cca = tx_task.cca;
        RadioTransition::new(
            self,
            tx_task,
            timed_txen,
            move || {
                let r = Self::radio();

                r.packetptr.write(|w| w.packetptr().variant(packetptr));

                Self::set_ifs(ifs);

                // NOTE: We need to set up shorts before completing the task to
                //       avoid race conditions, see RX_IDLE case below.
                if cca {
                    r.shorts.write(|w| {
                        if timed_txen.is_none() {
                            // Ramp down and up again for proper IFS and CCA
                            // timing.
                            w.end_disable().enabled();
                            w.disabled_rxen().enabled();
                            w.rxready_ccastart().enabled();
                        }
                        // If the channel is idle, then ramp up and start TX immediately.
                        w.ccaidle_txen().enabled();
                        w.txready_start().enabled();
                        // If the channel is busy, then disable the receiver so that
                        // we reach the fallback state.
                        w.ccabusy_disable().enabled()
                    });
                } else {
                    r.shorts.write(|w| {
                        if timed_txen.is_none() {
                            // Ramp down and directly switch to TX mode w/o CCA
                            // including IFS timing.
                            w.end_disable().enabled();
                        }
                        w.disabled_txen().enabled();
                        w.txready_start().enabled()
                    });
                }

                Ok(())
            },
            || {
                // Check whether the task completed before we were able to
                // automate the transition.
                //
                // NOTE: Read the state _after_ having set the shorts.
                let r = Self::radio();
                if r.state.read().state().is_rx_idle() {
                    // We're idle, although we have a short in place: This means
                    // that the previous packet was fully received before we
                    // were able to set the short.
                    r.tasks_disable.write(|w| w.tasks_disable().set_bit());
                }

                Ok(())
            },
            || {
                Self::radio().shorts.reset();
                Ok(())
            },
            rollback_on_crcerror,
        )
    }

    fn schedule_off(
        self,
        off_task: TaskOff,
        rollback_on_crcerror: bool,
    ) -> impl ExternalRadioTransition<NrfRadioDriver, TaskRx, TaskOff> {
        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::task_exec_begin(TASK_OFF_SCHEDULE);

        let timed_off = Self::timed_off(&off_task);
        RadioTransition::new(
            self,
            off_task,
            timed_off,
            || {
                // Ramp down the receiver.
                //
                // NOTE: We need to set up the short before completing the task
                //       to avoid race conditions, see RX_IDLE case below.
                //
                // NOTE: It's ok to leave the short on even in the timed case,
                //       as we don't have to schedule anything after the disable
                //       task.
                Self::radio().shorts.write(|w| w.end_disable().enabled());
                Ok(())
            },
            || {
                // Check whether the task completed before we were able to
                // automate the transition.
                //
                // NOTE: Read the state _after_ having set the short.
                let r = Self::radio();
                match r.state.read().state().variant() {
                    Some(STATE_A::RX_IDLE) => {
                        // We're idle, although we have a short in place: This
                        // means that the previous packet was fully received
                        // before we were able to set the short.
                        r.tasks_disable.write(|w| w.tasks_disable().set_bit());
                    }
                    Some(STATE_A::RX_DISABLE | STATE_A::DISABLED) => {}
                    _ => unreachable!(),
                };

                Ok(())
            },
            || {
                // Cleanup shorts.
                Self::radio().shorts.reset();
                Ok(())
            },
            rollback_on_crcerror,
        )
    }
}

/// Radio transmission state.
///
/// Entry: READY event
/// Exit: PHYEND event
///
/// State Invariants:
/// - The radio is in the TX or TXIDLE state.
/// - The radio's DMA pointer points to an empty packet in RAM.
/// - The "PHYEND" event has been cleared before starting reception.
/// - Only the "PHYEND" interrupt is enabled.
impl RadioDriver<NrfRadioDriver, TaskTx> {
    const fn timed_tx_to_rx(rx_task: &TaskRx) -> Option<TimedSignal> {
        if let Timestamp::Scheduled(rx_timestamp) = rx_task.start {
            // RMARKER offset: Tx -> Disabled -> Rx -> SHR
            const OFFSET_TX_TO_RX: SyntonizedDuration = T_TXDIS
                .checked_add(T_RXEN)
                .unwrap()
                .checked_add(T_SHR)
                .unwrap();
            Some(TimedSignal::new(
                rx_timestamp.checked_sub_duration(OFFSET_TX_TO_RX).unwrap(),
                HardwareSignal::RadioDisable,
            ))
        } else {
            None
        }
    }

    const fn timed_tx_to_tx(tx_task: &TaskTx) -> Option<TimedSignal> {
        if let Timestamp::Scheduled(tx_timestamp) = tx_task.at {
            let offset = if tx_task.cca {
                // RMARKER offset with CCA: Tx -> Disabled -> Rx -> CCA -> Turnaround -> SHR
                const OFFSET_TX_TO_TX_W_CCA: SyntonizedDuration = T_TXDIS
                    .checked_add(T_RXEN)
                    .unwrap()
                    .checked_add(T_CCA)
                    .unwrap()
                    .checked_add(T_TURNAROUND)
                    .unwrap()
                    .checked_add(T_SHR)
                    .unwrap();
                OFFSET_TX_TO_TX_W_CCA
            } else {
                // RMARKER offset without CCA: Tx -> Disabled -> Tx -> SHR
                const OFFSET_TX_TO_TX_NO_CCA: SyntonizedDuration = T_TXDIS
                    .checked_add(T_TXEN)
                    .unwrap()
                    .checked_add(T_SHR)
                    .unwrap();
                OFFSET_TX_TO_TX_NO_CCA
            };
            Some(TimedSignal::new(
                tx_timestamp.checked_sub_duration(offset).unwrap(),
                HardwareSignal::RadioDisable,
            ))
        } else {
            None
        }
    }
}

impl RadioState<TaskTx> for RadioDriver<NrfRadioDriver, TaskTx> {
    async fn transition(&mut self) -> Result<(), RadioTaskError<TaskTx>> {
        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::task_exec_begin(TASK_TRANSITION_TO_TX);

        let r = Self::radio();

        if let Some(tx_task) = &self.task {
            if tx_task.cca {
                if r.events_ccabusy.read().events_ccabusy().bit_is_set() {
                    r.events_ccabusy.reset();
                    let recovered_task = self.task.take().unwrap();
                    return Err(RadioTaskError::Task(TxError::CcaBusy(
                        recovered_task.radio_frame,
                    )));
                }

                r.events_ccaidle.reset();
            }
        }

        // Wait until the state enters.
        unsafe {
            self.inner
                .executor
                .spawn(poll_fn(|_| {
                    if r.events_txready.read().events_txready().bit_is_set() {
                        r.intenclr.write(|w| w.txready().set_bit());
                        r.events_txready.reset();
                        Poll::Ready(())
                    } else {
                        r.intenset.write(|w| w.txready().set_bit());
                        Poll::Pending
                    }
                }))
                .await;
        }

        Ok(())
    }

    async fn run(
        &mut self,
        timed_transition: Option<TimedSignal>,
        _: bool,
    ) -> Result<TxResult, RadioTaskError<TaskTx>> {
        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::task_exec_begin(TASK_TX_RUN);

        let r = Self::radio();

        // Wait until the task completed.
        if let Some(timed_transition) = timed_transition {
            let result = unsafe {
                self.timer()
                    .wait_until(timed_transition.instant, Some(timed_transition.signal))
                    .await
            };
            if matches!(result, RadioTimerResult::Overdue) {
                return Err(RadioTaskError::Scheduling(SchedulingError));
            }
        } else {
            unsafe {
                self.inner
                    .executor
                    .spawn(poll_fn(|_| {
                        if r.events_end.read().events_end().bit_is_set() {
                            r.intenclr.write(|w| w.end().set_bit());
                            Poll::Ready(())
                        } else {
                            r.intenset.write(|w| w.end().set_bit());
                            Poll::Pending
                        }
                    }))
                    .await;
            }
        }

        dma_end_fence();

        r.events_framestart.reset();

        let radio_frame = self.task.take().unwrap().radio_frame;
        if r.events_end.read().events_end().bit_is_set() {
            r.events_end.reset();
            Ok(TxResult::Sent(radio_frame))
        } else {
            Err(RadioTaskError::Task(TxError::Interrupted(radio_frame)))
        }
    }

    fn exit(&mut self) -> Result<(), SchedulingError> {
        Ok(())
    }
}

impl TxState<NrfRadioDriver> for RadioDriver<NrfRadioDriver, TaskTx> {
    fn schedule_rx(
        self,
        rx_task: TaskRx,
        ifs: Ifs,
    ) -> impl ExternalRadioTransition<NrfRadioDriver, TaskTx, TaskRx> {
        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::task_exec_begin(TASK_RX_SCHEDULE);

        let packetptr = rx_task.radio_frame.as_ptr() as u32;
        dma_start_fence();

        let timed_rx = Self::timed_tx_to_rx(&rx_task);
        RadioTransition::new(
            self,
            rx_task,
            timed_rx,
            move || {
                let r = Self::radio();

                r.packetptr.write(|w| w.packetptr().variant(packetptr));

                Self::set_ifs(ifs);

                // NOTE: To cater for errata 204 (see rev1 v1.4) a TX-to-RX
                //       switch must pass through the disabled state, which is
                //       what the shorts imply anyway.

                // NOTE: We need to set up the shorts before checking radio state to
                //       avoid race conditions, see the TX_IDLE case below.
                r.shorts.write(|w| {
                    // Ramp down the receiver, ramp it up again in RX mode and
                    // then start packet reception immediately.
                    if timed_rx.is_none() {
                        w.end_disable().enabled();
                    }
                    w.disabled_rxen().enabled();
                    w.rxready_start().enabled()
                });

                Ok(())
            },
            || {
                let r = Self::radio();

                // We need to schedule the BCSTART short _after_ the FRAMESTART
                // event of the TX packet, otherwise the TX packet will trigger
                // the BCMATCH event already.
                r.shorts.modify(|_, w| w.framestart_bcstart().enabled());

                // Check whether the task completed before we were able to
                // automate the transition.
                //
                // NOTE: Only read the state _after_ having set the short.
                if r.state.read().state().is_tx_idle() {
                    // We're idle, although we have a short in place: This means
                    // that the previous packet ended before we were able to set the
                    // short, i.e., hardware did not start transitioning to RX,
                    // we need to start it manually, see conditions 1. and 2. in the
                    // method documentation.
                    r.tasks_disable.write(|w| w.tasks_disable().set_bit());
                    debug!("late scheduling");
                };
                Ok(())
            },
            || {
                // Cleanup shorts. Don't reset to keep the bcmatch short
                // enabled.
                Self::radio().shorts.modify(|_, w| {
                    w.end_disable().disabled();
                    w.disabled_rxen().disabled();
                    w.rxready_start().disabled()
                });
                Ok(())
            },
            false,
        )
    }

    fn schedule_tx(
        self,
        mut tx_task: TaskTx,
        ifs: Ifs,
    ) -> impl SelfRadioTransition<NrfRadioDriver, TaskTx, TaskTx> {
        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::task_exec_begin(TASK_TX_SCHEDULE);

        let packetptr = prepare_tx_frame(&mut tx_task.radio_frame);
        dma_start_fence();

        let timed_tx = Self::timed_tx_to_tx(&tx_task);
        let cca = tx_task.cca;
        RadioTransition::new(
            self,
            tx_task,
            timed_tx,
            move || {
                // NOTE: We need to set up the shorts before checking radio state to
                //       avoid race conditions, see the TX_IDLE case below.
                let r = Self::radio();

                r.packetptr.write(|w| w.packetptr().variant(packetptr));

                Self::set_ifs(ifs);

                if cca {
                    r.shorts.write(|w| {
                        // Ramp down the transceiver, ramp it up again in RX mode and
                        // then start CCA immediately.
                        if timed_tx.is_none() {
                            w.end_disable().enabled();
                        }
                        w.disabled_rxen().enabled();
                        w.rxready_ccastart().enabled();

                        // If the channel is idle, then ramp up and start TX immediately.
                        w.ccaidle_txen().enabled();
                        w.txready_start().enabled();

                        // If the channel is busy, then disable the receiver.
                        w.ccabusy_disable().enabled()
                    });
                } else {
                    r.shorts.write(|w| {
                        // Ramp down and up again for proper IFS timing.
                        if timed_tx.is_none() {
                            w.end_disable().enabled();
                        }
                        w.disabled_txen().enabled();
                        w.txready_start().enabled()
                    })
                }

                Ok(())
            },
            || {
                // Check whether the task completed before we were able to
                // automate the transition.
                //
                // NOTE: Read the state _after_ having set the short.
                let r = Self::radio();
                if r.state.read().state().is_tx_idle() {
                    // We check whether a second packet was already sent
                    // just in the unlikely case that we got here so late
                    // that the next task was already executed.
                    if r.events_end.read().events_end().bit_is_clear() {
                        // We're idle, although we have a short in place:
                        // This means that the previous packet ended before
                        // we were able to set the short, i.e. transitioning
                        // to RX was not started by hardware, we need to
                        // start it manually, see conditions 1. and 2. in
                        // the method documentation.
                        r.tasks_disable.write(|w| w.tasks_disable().set_bit());
                        debug!("late scheduling");
                    } else {
                        debug!("slow completion");
                    }
                }

                Ok(())
            },
            || {
                // Clean up shorts
                Self::radio().shorts.reset();
                Ok(())
            },
            false,
        )
    }

    fn schedule_off(
        self,
        off_task: TaskOff,
    ) -> impl ExternalRadioTransition<NrfRadioDriver, TaskTx, TaskOff> {
        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::task_exec_begin(TASK_OFF_SCHEDULE);

        let timed_off = Self::timed_off(&off_task);
        RadioTransition::new(
            self,
            off_task,
            timed_off,
            move || {
                // Ramp down the receiver.
                //
                // NOTE: We need to set up the shorts before checking radio state to
                //       avoid race conditions, see TX_IDLE case below.
                //
                // NOTE: It's ok to leave the short on even in the timed case,
                //       as we don't have to schedule anything after the disable
                //       task.
                Self::radio().shorts.write(|w| w.end_disable().enabled());

                Ok(())
            },
            move || {
                // Check whether the transition has already triggered. If not then wait
                // until it triggers.
                //
                // NOTE: Read the state _after_ having set the short.
                let r = Self::radio();
                match r.state.read().state().variant() {
                    Some(STATE_A::TX_IDLE) => {
                        // We're idle, although we have a short in place: This
                        // means that the previous packet was either fully
                        // received before we were able to set the short or the
                        // RX window ended w/o receiving a packet. In any case
                        // disabling the radio was not started by hardware, we
                        // need to disable it manually.
                        r.tasks_disable.write(|w| w.tasks_disable().set_bit());
                    }
                    Some(STATE_A::TX | STATE_A::TX_DISABLE | STATE_A::DISABLED) => {}
                    _ => unreachable!(),
                };

                Ok(())
            },
            || {
                // Clean up shorts.
                Self::radio().shorts.reset();
                Ok(())
            },
            false,
        )
    }
}

fn prepare_tx_frame(radio_frame: &mut RadioFrame<RadioFrameSized>) -> u32 {
    let sdu_length = radio_frame.sdu_wo_fcs_length().get() as u8 + FCS_LEN as u8;
    // Set PHY HDR.
    radio_frame.pdu_mut()[0] = sdu_length;
    // Return PACKETPTR.
    radio_frame.as_ptr() as u32
}

/// This method must be called after all normal memory write accesses to the
/// buffer and before the volatile write operation passing the buffer pointer to
/// DMA hardware.
fn dma_start_fence() {
    // Note the explanation re using compiler fences with volatile accesses
    // rather than atomics in
    // <https://docs.rust-embedded.org/embedonomicon/dma.html>. The example
    // there is basically correct except that the fence should be placed before
    // passing the pointer to the hardware, not after.
    //
    // Other relevant sources:
    // - Interaction between volatile and fence:
    //   <https://github.com/rust-lang/unsafe-code-guidelines/issues/260>
    // - RFC re volatile access - including DMA discussions:
    //   <https://github.com/rust-lang/unsafe-code-guidelines/issues/321#issuecomment-2894697770>
    // - Compiler fence and DMA:
    //   <https://users.rust-lang.org/t/compiler-fence-dma/132027/39>
    // - asm! as memory barrier:
    //   <https://users.rust-lang.org/t/how-to-correctly-use-asm-as-memory-barrier-llvm-question/132105>
    // - Why asm! cannot (yet) be used as a barrier:
    //   <https://github.com/rust-lang/rust/issues/144351>
    compiler_fence(Ordering::Release);
}

/// This method must be called after any volatile read operation confirming that
/// the DMA has finished and before normal memory read accesses to the buffer.
fn dma_end_fence() {
    compiler_fence(Ordering::Acquire);
}
