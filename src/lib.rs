#![no_std]

mod bbq;
pub use bbq::{init, Consumer, DefmtConsumer, Error as BBQError, GrantR, SplitGrantR};
mod consts;
mod rtt;
use rtt::handle;

#[cfg(feature = "async-await")]
mod csec_waker;

use core::sync::atomic::{AtomicBool, Ordering};

#[defmt::global_logger]
struct Logger;

/// Global logger lock.
static TAKEN: AtomicBool = AtomicBool::new(false);
static mut CS_RESTORE: critical_section::RestoreState = critical_section::RestoreState::invalid();
static mut ENCODER: defmt::Encoder = defmt::Encoder::new();

fn combined_write(data: &[u8]) {
    rtt::do_write(data);
    bbq::do_write(data);
}

unsafe impl defmt::Logger for Logger {
    fn acquire() {
        // safety: Must be paired with corresponding call to release(), see below
        let restore = unsafe { critical_section::acquire() };

        // safety: accessing the `static mut` is OK because we have acquired a critical section.
        if TAKEN.load(Ordering::Relaxed) {
            panic!("defmt logger taken reentrantly")
        }

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
        // safety: accessing the `&'static _` is OK because we have acquired a critical section.
        handle().flush();
    }

    unsafe fn release() {
        // safety: accessing the `static mut` is OK because we have acquired a critical section.
        ENCODER.end_frame(combined_write);

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
        // Return early to avoid the encoder having to encode bytes we are going to throw away
        if bbq::check_latch(Ordering::Relaxed).is_err() {
            return;
        }

        // safety: accessing the `static mut` is OK because we have acquired a critical section.
        ENCODER.write(bytes, combined_write);
    }
}
