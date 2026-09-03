#![no_std]

mod consts;

#[cfg(feature = "bbq")]
mod bbq;
#[cfg(feature = "bbq")]
#[allow(deprecated)]
pub use bbq::internal_initialize;
#[cfg(feature = "bbq")]
pub use bbq::{DefmtConsumer, GrantR, InitError, ReadGrantError};

#[cfg(feature = "rtt")]
mod rtt;
#[cfg(feature = "rtt")]
use rtt::handle;

use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, Ordering},
};

#[cfg(not(any(feature = "rtt", feature = "bbq")))]
compile_error!("You must select at least one of the `rtt` or `bbq` features (or both).");

#[cfg(not(feature = "bbq"))]
#[macro_export]
macro_rules! init {
    ($path:path) => {
        Result::<(), ()>::Ok(())
    };
    () => {
        Result::<(), ()>::Ok(())
    };
}

#[defmt::global_logger]
struct Logger;

/// Global logger lock.
static TAKEN: AtomicBool = AtomicBool::new(false);
static mut CS_RESTORE: critical_section::RestoreState = critical_section::RestoreState::invalid();

struct EncoderCell(UnsafeCell<defmt::Encoder>);

// Access is serialized by the logger's critical section.
unsafe impl Sync for EncoderCell {}

static ENCODER: EncoderCell = EncoderCell(UnsafeCell::new(defmt::Encoder::new()));

fn combined_write(_data: &[u8]) {
    #[cfg(feature = "rtt")]
    rtt::do_write(_data);
    #[cfg(feature = "bbq")]
    bbq::do_write(_data);
}

unsafe impl defmt::Logger for Logger {
    fn acquire() {
        // safety: Must be paired with corresponding call to release(), see below
        let restore = unsafe { critical_section::acquire() };

        // safety: accessing the `static mut` is OK because we have acquired a critical section.
        if TAKEN.load(Ordering::Relaxed) {
            panic!("defmt logger taken reentrantly")
        }

        #[cfg(feature = "bbq")]
        if unsafe { bbq::ensure_initialized() }.is_err() {
            panic!("defmt_brtt is not initialized")
        }

        // safety: accessing the `static mut` is OK because we have acquired a critical section.
        TAKEN.store(true, Ordering::Relaxed);

        // safety: accessing the `static mut` is OK because we have acquired a critical section.
        unsafe { CS_RESTORE = restore };

        // safety: accessing the `static mut` is OK because we have acquired a critical section.
        unsafe { (*ENCODER.0.get()).start_frame(combined_write) }
    }

    unsafe fn flush() {
        #[cfg(feature = "rtt")]
        // safety: accessing the `&'static _` is OK because we have acquired a critical section.
        handle().flush();
    }

    unsafe fn release() {
        // safety: accessing the `static mut` is OK because we have acquired a critical section.
        (*ENCODER.0.get()).end_frame(combined_write);

        // safety: accessing the `static mut` is OK because we have acquired a critical section.
        TAKEN.store(false, Ordering::Relaxed);

        // safety: accessing the `static mut` is OK because we have acquired a critical section.
        let restore = CS_RESTORE;

        // safety: Must be paired with corresponding call to acquire(), see above
        critical_section::release(restore);

        // Wakers may execute arbitrary code, so wake only after leaving the logger lock.
        #[cfg(feature = "async-await")]
        bbq::wake_consumer();
    }

    unsafe fn write(bytes: &[u8]) {
        #[cfg(all(feature = "bbq", not(feature = "rtt")))]
        // Return early to avoid the encoder having to encode bytes we are going to throw away
        if unsafe { bbq::ensure_initialized() }.is_err() {
            return;
        }

        // safety: accessing the `static mut` is OK because we have acquired a critical section.
        (*ENCODER.0.get()).write(bytes, combined_write);
    }
}
