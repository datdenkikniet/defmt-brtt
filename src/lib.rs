#![no_std]

mod consts;

#[cfg(feature = "bbq")]
mod bbq;
#[cfg(feature = "bbq")]
pub use bbq::{init, Consumer, DefmtConsumer, Error as BBQError, GrantR, SplitGrantR};

#[cfg(feature = "rtt")]
mod rtt;
#[cfg(feature = "rtt")]
use rtt::handle;

#[cfg(feature = "async-await")]
mod csec_waker;

use core::sync::atomic::{AtomicBool, Ordering};

#[cfg(not(any(feature = "rtt", feature = "bbq")))]
compile_error!("You must select at least one of the `rtt` or `bbq` features (or both).");

#[defmt::global_logger]
struct Logger;

/// Global logger lock.
static TAKEN: AtomicBool = AtomicBool::new(false);
static mut CS_RESTORE: critical_section::RestoreState = critical_section::RestoreState::invalid();
static mut ENCODER: defmt::Encoder = defmt::Encoder::new();

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
        if bbq::should_bail() {
            // safety: we have acquired a critical section and are releasing it
            // again, without modifying the TAKEN variable.
            unsafe { critical_section::release(restore) };
            return;
        }

        // safety: accessing the `static mut` is OK because we have acquired a critical section.
        TAKEN.store(true, Ordering::Relaxed);

        // safety: accessing the `static mut` is OK because we have acquired a critical section.
        unsafe { CS_RESTORE = restore };

        // safety: accessing the `static mut` is OK because we have acquired a critical section.
        unsafe { ENCODER.start_frame(combined_write) }
    }

    unsafe fn flush() {
        #[cfg(feature = "rtt")]
        // safety: accessing the `&'static _` is OK because we have acquired a critical section.
        handle().flush();
    }

    unsafe fn release() {
        // safety: accessing the `static mut` is OK because we have acquired a critical section.
        ENCODER.end_frame(combined_write);

        #[cfg(feature = "bbq")]
        bbq::commit_w_grant();

        // safety: accessing the `static mut` is OK because we have acquired a critical section.
        TAKEN.store(false, Ordering::Relaxed);

        // safety: accessing the `static mut` is OK because we have acquired a critical section.
        let restore = CS_RESTORE;

        // safety: Must be paired with corresponding call to acquire(), see above
        critical_section::release(restore);

        // Wake the defmt consumer's waker
        #[cfg(feature = "async-await")]
        bbq::DefmtConsumer::waker().wake();
    }

    unsafe fn write(bytes: &[u8]) {
        #[cfg(feature = "bbq")]
        // Return early to avoid the encoder having to encode bytes we are going to throw away
        if bbq::check_latch(Ordering::Relaxed).is_err() {
            return;
        }

        // safety: accessing the `static mut` is OK because we have acquired a critical section.
        ENCODER.write(bytes, combined_write);
    }
}
