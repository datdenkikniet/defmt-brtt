use core::cell::UnsafeCell;

#[cfg(feature = "async-await")]
use core::cell::Cell;
#[cfg(feature = "async-await")]
use critical_section::Mutex;

use bbqueue::prod_cons::stream::{StreamConsumer, StreamGrantR};
pub use bbqueue::traits::coordination::ReadGrantError;

#[cfg(not(feature = "async-await"))]
use bbqueue::nicknames::Jerk;
#[cfg(feature = "async-await")]
use bbqueue::{
    export::ConstInit,
    nicknames::Memphis,
    traits::notifier::{maitake::MaiNotSpsc, AsyncNotifier, Notifier},
};

/// BBQueue buffer size. Default: 1024; can be customized by setting the
/// `DEFMT_BRTT_BUFFER_SIZE` environment variable at compile time.
pub use crate::consts::BUF_SIZE;

#[cfg(not(feature = "async-await"))]
type Queue = Jerk<BUF_SIZE>;
#[cfg(feature = "async-await")]
type Queue = Memphis<BUF_SIZE, DeferredNotifier>;

/// A contiguous view of committed logging data.
pub type GrantR = StreamGrantR<&'static Queue>;

/// An error returned while initializing the BBQueue transport.
#[derive(Debug, defmt::Format, PartialEq, Eq)]
pub enum InitError {
    /// The BBQueue transport has already been initialized.
    AlreadyInitialized,
    /// A log was emitted before the BBQueue transport was initialized.
    UseBeforeInit,
}

/// Initialize the BBQueue-based global defmt sink.
///
/// This must be called before the first defmt log. The first call returns the
/// consumer; subsequent calls return [`InitError::AlreadyInitialized`].
#[macro_export]
macro_rules! init {
    ($path:path) => {{
        #[allow(deprecated)]
        $path::internal_initialize()
    }};
    () => {{
        #[allow(deprecated)]
        ::defmt_brtt::internal_initialize()
    }};
}

/// Initialize the BBQueue transport.
///
/// Use the [`init!`](crate::init) macro instead so renamed crate dependencies
/// continue to work.
#[deprecated(note = "Use the `init` macro instead.")]
#[doc(hidden)]
pub fn internal_initialize() -> Result<DefmtConsumer, InitError> {
    critical_section::with(|_| {
        // Safety: all state access is serialized by this critical section.
        let state = unsafe { &mut *BRTT_INITIALIZED.0.get() };
        match *state {
            state::UNINITIALIZED => {
                *state = state::INITIALIZED;
                Ok(DefmtConsumer {
                    consumer: BBQ.stream_consumer(),
                })
            }
            state::INITIALIZED => Err(InitError::AlreadyInitialized),
            _ => Err(InitError::UseBeforeInit),
        }
    })
}

/// Consumer for the encoded defmt byte stream.
pub struct DefmtConsumer {
    consumer: StreamConsumer<&'static Queue>,
}

impl DefmtConsumer {
    /// Obtain a contiguous slice of committed bytes.
    ///
    /// The slice may not contain all available bytes when the queue wraps.
    pub fn read(&mut self) -> Result<GrantR, ReadGrantError> {
        self.consumer.read()
    }

    /// Wait until logging data is available and return a contiguous grant.
    ///
    /// The grant may not contain all available bytes when the queue wraps.
    #[cfg(feature = "async-await")]
    pub fn wait_for_log(&mut self) -> impl core::future::Future<Output = GrantR> + Send + '_ {
        self.consumer.wait_read()
    }
}

mod state {
    pub const UNINITIALIZED: u8 = 0;
    pub const INITIALIZED: u8 = 1;
    pub const USE_BEFORE_INIT: u8 = 2;
}

static BBQ: Queue = Queue::new();

#[repr(transparent)]
struct StateCell(UnsafeCell<u8>);

// State access is serialized by the logger or initialization critical section.
unsafe impl Sync for StateCell {}

#[no_mangle]
static BRTT_INITIALIZED: StateCell = StateCell(UnsafeCell::new(state::UNINITIALIZED));

/// Check initialization while the logger's critical section is held.
///
/// # Safety
///
/// The caller must hold a critical section for the entire call to serialize
/// access to `BRTT_INITIALIZED` with initialization and other logger calls.
/// No other reference to the initialization state may be live during this call.
pub(crate) unsafe fn ensure_initialized() -> Result<(), InitError> {
    // SAFETY: the caller holds a critical section and guarantees exclusive
    // access to the initialization state for the duration of this call.
    let state = unsafe { &mut *BRTT_INITIALIZED.0.get() };
    match *state {
        state::INITIALIZED => Ok(()),
        state::UNINITIALIZED => {
            *state = state::USE_BEFORE_INIT;
            Err(InitError::UseBeforeInit)
        }
        _ => Err(InitError::UseBeforeInit),
    }
}

#[cfg(feature = "async-await")]
#[doc(hidden)]
pub struct DeferredNotifier;

#[cfg(feature = "async-await")]
static ASYNC_NOTIFIER: MaiNotSpsc = MaiNotSpsc::new();
#[cfg(feature = "async-await")]
static CONSUMER_WAKE_PENDING: Mutex<Cell<bool>> = Mutex::new(Cell::new(false));

#[cfg(feature = "async-await")]
impl ConstInit for DeferredNotifier {
    const INIT: Self = Self;
}

#[cfg(feature = "async-await")]
impl Notifier for DeferredNotifier {
    fn wake_one_consumer(&self) {
        critical_section::with(|cs| CONSUMER_WAKE_PENDING.borrow(cs).set(true));
    }

    fn wake_one_producer(&self) {
        ASYNC_NOTIFIER.wake_one_producer();
    }
}

#[cfg(feature = "async-await")]
impl AsyncNotifier for DeferredNotifier {
    async fn wait_for_not_empty<T, F: FnMut() -> Option<T>>(&self, f: F) -> T {
        ASYNC_NOTIFIER.wait_for_not_empty(f).await
    }

    async fn wait_for_not_full<T, F: FnMut() -> Option<T>>(&self, f: F) -> T {
        ASYNC_NOTIFIER.wait_for_not_full(f).await
    }
}

#[cfg(feature = "async-await")]
pub(crate) fn wake_consumer() {
    let pending = critical_section::with(|cs| CONSUMER_WAKE_PENDING.borrow(cs).replace(false));
    if pending {
        ASYNC_NOTIFIER.wake_one_consumer();
    }
}

/// Drain as many bytes to the queue as possible. If it fills, discard the
/// remaining bytes as required by the best-effort defmt logger contract.
pub(crate) fn do_write(mut remaining: &[u8]) {
    let producer = BBQ.stream_producer();

    while !remaining.is_empty() {
        let Ok(mut grant) = producer.grant_max_remaining(remaining.len()) else {
            return;
        };

        let used = grant.len().min(remaining.len());
        grant[..used].copy_from_slice(&remaining[..used]);
        grant.commit(used);
        remaining = &remaining[used..];
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;

    struct TestCriticalSection;

    critical_section::set_impl!(TestCriticalSection);

    unsafe impl critical_section::Impl for TestCriticalSection {
        unsafe fn acquire() -> critical_section::RawRestoreState {}

        unsafe fn release(_: critical_section::RawRestoreState) {}
    }

    #[test]
    fn initializes_once_and_transfers_bytes() {
        let mut consumer = internal_initialize().unwrap();

        do_write(b"defmt");
        let grant = consumer.read().unwrap();
        assert_eq!(&*grant, b"defmt");
        grant.release(5);

        assert_eq!(
            internal_initialize().err(),
            Some(InitError::AlreadyInitialized)
        );
    }
}
