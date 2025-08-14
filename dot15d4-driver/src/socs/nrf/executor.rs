#![allow(static_mut_refs)]

#[cfg(feature = "gpio-trace")]
use core::cell::Cell;
use core::{
    cell::UnsafeCell,
    future::{poll_fn, Future},
    pin::{pin, Pin},
    ptr::{self, null_mut},
    sync::atomic::compiler_fence,
    task::{Context, Poll, RawWaker, Waker},
};

use cortex_m::asm::wfe;
use dot15d4_util::sync::CancellationGuard;
use nrf52840_hal::pac::{Interrupt, NVIC};
#[cfg(feature = "gpio-trace")]
use nrf52840_hal::pac::{Peripherals, GPIOTE};
use portable_atomic::{AtomicPtr, Ordering};

#[cfg(feature = "rtos-trace")]
use crate::executor::trace::MISSED_ISR;

// Safety: While the task pointer is `null` the scheduling context may mutate
//         the outer waker. If the task pointer is non-null then the outer_waker
//         may be read by the interrupt context. The getter and setter
//         implementations for the task pointer must ensure proper compiler
//         fencing.
struct State {
    // The task pointer points to the inner future to be executed. The task
    // pointer will be modified from both, scheduling and interrupt context.
    task_ptr: AtomicPtr<Pin<&'static mut dyn Future<Output = ()>>>,

    // The outer waker will only be present when spawning a task from within
    // another task (nested executors). The outer waker will only be modified
    // from scheduling context and then be released read-only to both,
    // scheduling and interrupt context.
    outer_waker: UnsafeCell<Option<Waker>>,

    // The inner waker is immutable. It may be accessed from interrupt context
    // only.
    inner_waker: Waker,

    // The interrupt is immutable. It may be read from any context.
    interrupt: Interrupt,

    // The GPIOTE trace channel must be set before instantiating the executor
    // and is immutable afterwards.
    #[cfg(feature = "gpio-trace")]
    gpiote_trace_channel: Cell<usize>,
}

impl State {
    const fn new(interrupt: Interrupt, raw_inner_waker: RawWaker) -> Self {
        Self {
            task_ptr: AtomicPtr::new(null_mut()),
            outer_waker: UnsafeCell::new(None),
            inner_waker: unsafe { Waker::from_raw(raw_inner_waker) },
            interrupt,
            #[cfg(feature = "gpio-trace")]
            gpiote_trace_channel: Cell::new(0),
        }
    }

    /// This method unmasks the interrupt and therefore may break concurrent
    /// critical sections. It must be called in early initialization code before
    /// concurrent critical sections might be active.
    ///
    /// Using the executor w/o calling `init()` will cause undefined behavior.
    fn init(&self, _gpiote_trace_channel: usize) {
        #[cfg(feature = "rtos-trace")]
        crate::executor::trace::instrument();

        #[cfg(feature = "gpio-trace")]
        self.gpiote_trace_channel.set(_gpiote_trace_channel);

        NVIC::unpend(self.interrupt);
        // Safety: See method doc. There should be no concurrent critical sections.
        unsafe { NVIC::unmask(self.interrupt) };
    }

    fn set_task_ptr(&self, task_ptr: *mut Pin<&'static mut dyn Future<Output = ()>>) {
        compiler_fence(Ordering::Release);
        self.task_ptr.store(task_ptr, Ordering::Relaxed);
    }

    fn task_ptr(&self) -> *mut Pin<&'static mut dyn Future<Output = ()>> {
        let task_ptr = self.task_ptr.load(Ordering::Relaxed);
        compiler_fence(Ordering::Acquire);
        task_ptr
    }

    // Safety: Must only be called from scheduling context and requires the
    //         outer waker to be acquired to the scheduling context (i.e. the
    //         task ptr to be null).
    unsafe fn set_outer_waker(&self, waker: Waker) {
        let outer_waker = unsafe { self.outer_waker.get().as_mut() }.unwrap();
        *outer_waker = Some(waker);
    }

    // Safety: Must only be called from scheduling context and requires the
    //         outer waker to be acquired to the scheduling context (i.e. the
    //         task ptr to be null).
    unsafe fn clear_outer_waker(&self) {
        let outer_waker = unsafe { self.outer_waker.get().as_mut() }.unwrap();
        *outer_waker = None;
    }

    // Safety: May be accessed from both, interrupt and scheduling context.
    unsafe fn outer_waker(&self) -> &Option<Waker> {
        unsafe { self.outer_waker.get().as_ref() }.unwrap()
    }

    fn pend_interrupt(&self) {
        // Safety: Triggering a task is atomic and idempotent.
        NVIC::pend(self.interrupt);
    }

    fn on_interrupt(&self) {
        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::isr_enter();

        #[cfg(feature = "gpio-trace")]
        let gpiote = {
            // Safety: The GPIOTE trace channel is reserved for exclusive use.
            let gpiote = unsafe { Peripherals::steal() }.GPIOTE;
            gpiote.tasks_out[self.gpiote_trace_channel.get()].write(|w| w.tasks_out().set_bit());
            gpiote
        };

        let task = self.task_ptr();

        // Safety: We're converting from a pointer that has been generated verbatim
        //         from a valid &mut reference. Therefore the pointer will be
        //         properly aligned, dereferenceable and point to a valid pinned
        //         future. Pinning and synchronizing via the pointer ensures that
        //         the pointer cannot dangle. Checking for null pointers is
        //         required to protect against spurious wake-ups.
        if let Some(task) = unsafe { task.as_mut() } {
            // The inner waker is shared immutably.
            let mut cx = Context::from_waker(&self.inner_waker);

            // Safety: A non-null task pointer indicates that the interrupt
            //         temporarily owns the task and outer waker.
            match task.as_mut().poll(&mut cx) {
                core::task::Poll::Ready(_) => {
                    if let Some(outer_waker_ref) = unsafe { self.outer_waker() } {
                        outer_waker_ref.wake_by_ref();
                    }

                    // Safety: Setting the task pointer to null hands ownership over
                    //         task and outer waker back to the scheduling context.
                    self.set_task_ptr(null_mut());
                }
                core::task::Poll::Pending => {}
            }
        } else {
            #[cfg(feature = "rtos-trace")]
            rtos_trace::trace::marker(MISSED_ISR);
        }

        #[cfg(feature = "gpio-trace")]
        gpiote.tasks_out[self.gpiote_trace_channel.get()].write(|w| w.tasks_out().set_bit());

        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::isr_exit_to_scheduler();
    }

    fn block_on<Task: Future<Output = ()>>(&self, task: Task) {
        debug_assert!(NVIC::is_enabled(self.interrupt), "not initialized");

        // Safety: The pinned task must not move while the interrupt accesses
        //         it. Note that this is not automatically enforced by Pin (as
        //         the pointee may be Unpin).
        let mut pinned_task: Pin<&mut dyn Future<Output = ()>> = pin!(task);

        // Safety: We may cast to static lifetime as our implementation ensures
        //         that the pointer never dangles when accessed (see the drop
        //         policy below). Storing the task pointer releases the pinned
        //         task to interrupt context.
        self.set_task_ptr(ptr::from_mut(&mut pinned_task).cast());

        // Initially poll once.
        self.pend_interrupt();

        loop {
            #[cfg(feature = "rtos-trace")]
            rtos_trace::trace::system_idle();
            wfe();

            // Safety: Loading the task pointer re-acquires the pinned task for
            //         dropping.
            if self.task_ptr().is_null() {
                // Safety: We need to extend lifetime until we're sure the task
                //         is no longer being accessed.
                #[allow(clippy::drop_non_drop)]
                drop(pinned_task);
                break;
            }
        }
    }

    async unsafe fn spawn<Task: Future<Output = ()>>(&self, task: Task) {
        debug_assert!(NVIC::is_enabled(self.interrupt), "not initialized");

        // Safety: The pinned task must not move while the interrupt accesses
        //         it. Note that this is not automatically enforced by Pin (as
        //         the pointee may be Unpin).
        let mut pinned_task: Pin<&mut dyn Future<Output = ()>> = pin!(task);

        let cancellation_guard = CancellationGuard::new(|| {
            // Safety: The interrupt checks the task pointer and then runs
            //         atomically from the perspective of the scheduling
            //         context. So it's ok to drop this task at any point from
            //         the scheduling context.
            self.set_task_ptr(null_mut());
            // Safety: Setting the task pointer to null acquired the outer
            //         waker.
            unsafe { self.clear_outer_waker() };
        });

        poll_fn(|cx| {
            // Safety: Loading the task pointer (re-)acquires the outer waker's
            //         memory.
            let task_ptr = self.task_ptr();
            let outer_waker = unsafe { self.outer_waker() };

            if let Some(outer_waker_ref) = outer_waker {
                if task_ptr.is_null() {
                    // A task pointer value of null while a waker is present
                    // signals that the task finished.

                    // Safety: Loading a null value for the task pointer
                    //         re-acquires ownership of both, task and outer
                    //         waker, to the scheduling context so they can be
                    //         dropped. Also note that we asserted that the
                    //         (immutable) waker is "some", so no need to
                    //         re-check here.
                    unsafe { self.clear_outer_waker() };

                    Poll::Ready(())
                } else {
                    // The interrupt never wakes an unfinished task, therefore a
                    // non-null task pointer value while a waker is present
                    // signals an unsolicited poll by the outer executor, e.g.
                    // when racing concurrent tasks in a select primitive.

                    // Safety: While the interrupt is active we only have shared
                    //         read-only access to the waker. We therefore do
                    //         not support migration of the task.
                    debug_assert!(outer_waker_ref.will_wake(cx.waker()));

                    Poll::Pending
                }
            } else {
                // We're polled for the first time...
                debug_assert!(task_ptr.is_null());

                // Safety: While the task pointer is null, the scheduling
                //         context exclusively owns the outer waker for
                //         modification.
                unsafe { self.set_outer_waker(cx.waker().clone()) };

                // Setting the task pointer releases both, task and waker,
                // to the interrupt context.
                self.set_task_ptr(ptr::from_mut(&mut pinned_task).cast());

                // Safety: Once the interrupt context owns task and waker, it is
                //         safe to trigger it.
                self.pend_interrupt();

                Poll::Pending
            }
        })
        .await;

        cancellation_guard.inactivate();

        // Safety: We need to extend lifetime until we're sure the task
        //         is no longer being accessed.
        #[allow(clippy::drop_non_drop)]
        drop(pinned_task);
    }
}

// Safety: We synchronize the contents of state via the task atomic.
unsafe impl Sync for State {}

macro_rules! nrf_interrupt_executor {
    ($mod:ident, $interrupt:ident, $peripheral:ident) => {
        pub mod $mod {
            use core::{
                future::Future,
                ptr,
                task::{RawWaker, RawWakerVTable},
            };
            use nrf52840_hal::pac::{interrupt, $peripheral, Interrupt};
            use static_cell::StaticCell;

            use $crate::{executor::InterruptExecutor, interrupt_executor};

            use super::State;

            static VTABLE: RawWakerVTable = interrupt_executor!(NrfInterruptExecutor);
            static STATE: State =
                State::new(Interrupt::$interrupt, RawWaker::new(ptr::null(), &VTABLE));

            #[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Eq, Ord)]
            pub struct NrfInterruptExecutor;

            impl NrfInterruptExecutor {
                /// Mutability proves exclusive ownership of the executor.
                /// Ownership of the software interrupt is transferred to the
                /// executor.
                pub(super) fn init(
                    &'static mut self,
                    _peripheral: $peripheral,
                    gpiote_trace_channel: usize,
                ) -> &'static mut Self {
                    STATE.init(gpiote_trace_channel);
                    self
                }
            }

            impl InterruptExecutor for NrfInterruptExecutor {
                fn block_on<Task: Future<Output = ()>>(&mut self, task: Task) {
                    STATE.block_on(task);
                }

                async unsafe fn spawn<Task: Future<Output = ()>>(&mut self, task: Task) {
                    STATE.spawn(task).await;
                }

                fn pend_interrupt() {
                    STATE.pend_interrupt();
                }
            }

            #[interrupt]
            fn $interrupt() {
                STATE.on_interrupt();
            }

            pub(super) static EXECUTOR: StaticCell<NrfInterruptExecutor> = StaticCell::new();
        }

        /// Safety: Transferring ownership of the interrupt peripheral proves
        ///         that only a single instance of the executor can be
        ///         requested.
        #[cfg(not(feature = "gpio-trace"))]
        pub fn $mod(
            peripheral: nrf52840_hal::pac::$peripheral,
        ) -> &'static mut $mod::NrfInterruptExecutor {
            $mod::EXECUTOR
                .init($mod::NrfInterruptExecutor)
                .init(peripheral, 0)
        }

        #[cfg(feature = "gpio-trace")]
        pub fn $mod(
            peripheral: nrf52840_hal::pac::$peripheral,
            _gpiote: &GPIOTE,
            gpiote_trace_channel: usize,
        ) -> &'static mut $mod::NrfInterruptExecutor {
            $mod::EXECUTOR
                .init($mod::NrfInterruptExecutor)
                .init(peripheral, gpiote_trace_channel)
        }
    };
}

nrf_interrupt_executor!(swi0, SWI0_EGU0, SWI0);
nrf_interrupt_executor!(radio, RADIO, RADIO);
