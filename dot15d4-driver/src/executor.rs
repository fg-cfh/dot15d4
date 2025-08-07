use core::future::Future;

/// A specialized executor meant as a drop-in replacement for a
/// conventional interrupt handler inside drivers.
///
/// Implementations of this executor SHALL be optimized for low-latency,
/// low-jitter, deterministic wake-to-poll times. The system SHOULD be placed in
/// a low-energy mode while the inner future is pending.
///
/// Objectives:
/// - fast wake-to-poll times compared to e.g. RTIC or embassy's executor
/// - deterministic behavior with no measurable jitter for timing-critical tasks
///   inside the driver while still being able to run higher-level application
///   code from any generic executor.
pub trait InterruptExecutor {
    /// Associates a task with the executor and drives it to completion.
    ///
    /// Use this from your main function or any other non-async function to
    /// block until the task finishes.
    ///
    /// Requiring a mutable reference ensures that only a single task can be
    /// scheduled at any time.
    fn block_on<Task: Future<Output = ()>>(&mut self, task: Task);

    /// Associates a task with the executor and returns a future that can be
    /// polled by a higher-level executor.
    ///
    /// Use this from any asynchronous function or spawn it as a task from your
    /// application-level executor. The returned future will be woken once the
    /// inner future has been driven to completion via interrupt callbacks.
    ///
    /// This allows you to nest executors without the overhead of deeply nested
    /// futures and without the latency and jitter of an application executor
    /// running at low priority.
    ///
    /// Requiring a mutable reference ensures that only a single task can be
    /// scheduled at any time.
    fn spawn<Task: Future<Output = ()>>(&mut self, task: Task) -> impl Future<Output = ()>;

    /// Pends the interrupt backing the interrupt executor.
    ///
    /// This is called internally by `wake()` and `wake_by_ref()`. Don't call
    /// this directly.
    fn pend_interrupt();
}

#[macro_export]
macro_rules! interrupt_executor {
    ($interrupt_executor:ty) => {{
        use core::task::{RawWaker, RawWakerVTable};

        unsafe fn clone_waker(data: *const ()) -> RawWaker {
            // Safety: We always return the same (static) vtable reference to ensure
            //         that `Waker::will_wake()` recognizes the clone.
            RawWaker::new(data, &VTABLE)
        }

        unsafe fn wake(_: *const ()) {
            <$interrupt_executor as InterruptExecutor>::pend_interrupt()
        }

        unsafe fn wake_by_ref(_: *const ()) {
            <$interrupt_executor as InterruptExecutor>::pend_interrupt()
        }

        unsafe fn drop_waker(_: *const ()) {
            // no-op
        }

        RawWakerVTable::new(clone_waker, wake, wake_by_ref, drop_waker)
    }};
}

#[cfg(feature = "rtos-trace")]
pub(crate) mod trace {
    // Tasks
    pub const TASK_INT: u32 = 2000;

    pub fn instrument() {
        rtos_trace::trace::task_new_stackless(TASK_INT, "IntExecutor\0", 0);
    }
}
