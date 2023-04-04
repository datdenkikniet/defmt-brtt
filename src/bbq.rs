use bbqueue::{BBBuffer, GrantW, Producer};
use core::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::atomic::{AtomicU8, AtomicUsize, Ordering},
};

#[cfg(feature = "async-await")]
use crate::csec_waker::CriticalSectionWakerRegistration as Waker;

/// BBQueue buffer size. Default: 1024; can be customized by setting the
/// `DEFMT_BRTT_BUFFER_SIZE` environment variable at compile time
pub use crate::consts::BUF_SIZE;

/// The `defmt-brtt` BBQueue error type
#[derive(Debug, defmt::Format, PartialEq, Eq)]
pub enum Error {
    /// An internal latching fault has occured. No more logs will be returned.
    /// This indicates a coding error in `defmt-bbq`. Please open an issue.
    InternalLatchingFault,

    /// The user attempted to log before initializing the `defmt-bbq` structure.
    /// This is a latching fault, and not recoverable, but is not an indicator of
    /// a failure in the library. If you see this error, please ensure that you
    /// call `defmt-bbq::init()` before making any defmt logging statements.
    UseBeforeInitLatchingFault,

    /// This indicates some potentially recoverable bbqerror (including no
    /// data currently available).
    Bbq(BBQError),
}

impl From<BBQError> for Error {
    fn from(other: BBQError) -> Self {
        Error::Bbq(other)
    }
}

/// This is the consumer type given to the user to drain the logging queue.
///
/// It is a re-export of the [bbqueue::Consumer](https://docs.rs/bbqueue/latest/bbqueue/struct.Consumer.html) type.
///
pub use bbqueue::Consumer;

/// An error returned by the underlying bbqueue storage
///
/// It is a re-export of the [bbqueue::Error](https://docs.rs/bbqueue/latest/bbqueue/enum.Error.html) type.
///
pub use bbqueue::Error as BBQError;

/// This is the Reader Grant type given to the user as a view of the logging queue.
///
/// It is a re-export of the [bbqueue::GrantR](https://docs.rs/bbqueue/latest/bbqueue/struct.GrantR.html) type.
///
pub use bbqueue::GrantR;

/// This is the Split Reader Grant type given to the user as a view of the logging queue.
///
/// It is a re-export of the [bbqueue::SplitGrantR](https://docs.rs/bbqueue/latest/bbqueue/struct.SplitGrantR.html) type.
///
pub use bbqueue::SplitGrantR;

// ----------------------------------------------------------------------------
// init() function - this is the (only) user facing interface
// ----------------------------------------------------------------------------

/// Initialize the BBQueue based global defmt sink. MUST be called before
/// the first `defmt` log, or it will latch a fault.
///
/// On the first call to this function, the Consumer end of the logging
/// queue will be returned. On any subsequent call, an error will be
/// returned.
///
/// For more information on the Consumer interface, see the [Consumer docs] in the `bbqueue`
/// crate documentation.
///
/// [Consumer docs]: https://docs.rs/bbqueue/latest/bbqueue/struct.Consumer.html
pub fn init() -> Result<DefmtConsumer, Error> {
    let (prod, cons) = BBQ.try_split()?;

    // NOTE: We are okay to treat the following as safe, as the BBQueue
    // split operation is guaranteed to only return Ok once in a
    // thread-safe manner.
    unsafe {
        BBQ_PRODUCER.uc_mu_fp.get().write(MaybeUninit::new(prod));
    }

    // MUST be done LAST
    BBQ_STATE.store(logstate::INIT_NO_STORED_GRANT, Ordering::Release);

    Ok(DefmtConsumer { cons })
}

// ----------------------------------------------------------------------------
// Defmt Consumer
// ----------------------------------------------------------------------------

/// The consumer interface of `defmt-log`.
///
/// This type is a wrapper around the
/// [bbqueue::Consumer](https://docs.rs/bbqueue/latest/bbqueue/struct.Consumer.html) type,
/// and returns defmt-brtt's [Error](crate::bbq::Error) type instead of bbqueue's
/// [bbqueue::Error](https://docs.rs/bbqueue/latest/bbqueue/enum.Error.html) type.
pub struct DefmtConsumer {
    cons: Consumer<'static, BUF_SIZE>,
}

impl DefmtConsumer {
    /// Obtains a contiguous slice of committed bytes. This slice may not
    /// contain ALL available bytes, if the writer has wrapped around. The
    /// remaining bytes will be available after all readable bytes are
    /// released.
    pub fn read<'a>(&'a mut self) -> Result<GrantR<'a, BUF_SIZE>, Error> {
        self.read_static()
    }

    fn read_static(&mut self) -> Result<GrantR<'static, BUF_SIZE>, Error> {
        Ok(self.cons.read()?)
    }

    /// Obtains two disjoint slices, which are each contiguous of committed bytes.
    /// Combined these contain all previously commited data at the time of read
    pub fn split_read(&mut self) -> Result<SplitGrantR<'static, BUF_SIZE>, Error> {
        Ok(self.cons.split_read()?)
    }
}

#[cfg(feature = "async-await")]
impl DefmtConsumer {
    pub(crate) fn waker() -> &'static Waker {
        static WAKER: Waker = Waker::new();
        &WAKER
    }

    pub async fn wait_for_log<'a>(&'a mut self) -> GrantR<'a, BUF_SIZE> {
        let mut polled_once = false;

        loop {
            let awaited_grant = core::future::poll_fn(|ctx| {
                Self::waker().register(ctx.waker());

                let grant = self.read_static();

                if let Ok(grant) = grant {
                    core::task::Poll::Ready(Some(grant))
                } else if !polled_once {
                    polled_once = true;
                    core::task::Poll::Pending
                } else {
                    core::task::Poll::Ready(None)
                }
            })
            .await;

            if let Some(grant) = awaited_grant.or(self.read_static().ok()) {
                break grant;
            }
        }
    }
}

// ----------------------------------------------------------------------------
// logstate, and state helper functions
// ----------------------------------------------------------------------------

mod logstate {
    // BBQ has NOT been initialized
    // BBQ_PRODUCER has NOT been initialized
    // BBQ_GRANT has NOT been initialized
    pub const UNINIT: u8 = 0;

    // BBQ HAS been initialized
    // BBQ_PRODUCER HAS been initialized
    // BBQ_GRANT has NOT been initialized
    pub const INIT_NO_STORED_GRANT: u8 = 1;

    // BBQ HAS been initialized
    // BBQ_PRODUCER HAS been initialized
    // BBQ_GRANT HAS been initialized
    pub const INIT_GRANT_IS_STORED: u8 = 2;

    // All state codes above 100 are a latching fault

    // A latching fault has occurred.
    pub const LATCH_INTERNAL_ERROR: u8 = 100;

    // The user attempted to log before init
    pub const LATCH_USE_BEFORE_INIT: u8 = 101;
}

#[inline]
pub(crate) fn check_latch(ordering: Ordering) -> Result<(), Error> {
    match BBQ_STATE.load(ordering) {
        i if i < logstate::LATCH_INTERNAL_ERROR => Ok(()),
        logstate::LATCH_USE_BEFORE_INIT => Err(Error::UseBeforeInitLatchingFault),
        _ => Err(Error::InternalLatchingFault),
    }
}

#[inline]
fn latch_assert_eq<T: PartialEq>(left: T, right: T) -> Result<(), Error> {
    if left == right {
        Ok(())
    } else {
        BBQ_STATE.store(logstate::LATCH_INTERNAL_ERROR, Ordering::Release);
        Err(Error::InternalLatchingFault)
    }
}

// ----------------------------------------------------------------------------
// UnsafeProducer
// ----------------------------------------------------------------------------

/// A storage structure for holding the maybe initialized producer with inner mutability
struct UnsafeProducer {
    uc_mu_fp: UnsafeCell<MaybeUninit<Producer<'static, BUF_SIZE>>>,
}

impl UnsafeProducer {
    const fn new() -> Self {
        Self {
            uc_mu_fp: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    // TODO: Could be made safe if we ensure the reference is only taken
    // once. For now, leave unsafe
    unsafe fn get_mut(&self) -> Result<&mut Producer<'static, BUF_SIZE>, Error> {
        latch_assert_eq(
            logstate::INIT_NO_STORED_GRANT,
            BBQ_STATE.load(Ordering::Relaxed),
        )?;

        // NOTE: `UnsafeCell` and `MaybeUninit` are both `#[repr(Transparent)],
        // meaning this direct cast is acceptable
        let const_ptr: *const Producer<'static, BUF_SIZE> = self.uc_mu_fp.get().cast();
        let mut_ptr: *mut Producer<'static, BUF_SIZE> = const_ptr as *mut _;
        let ref_mut: &mut Producer<'static, BUF_SIZE> = &mut *mut_ptr;

        Ok(ref_mut)
    }
}

unsafe impl Sync for UnsafeProducer {}

// ----------------------------------------------------------------------------
// UnsafeGrantW
// ----------------------------------------------------------------------------

struct UnsafeGrantW {
    uc_mu_fgw: UnsafeCell<MaybeUninit<GrantW<'static, BUF_SIZE>>>,

    /// Note: This stores the offset into the *current grant*, IFF a grant
    /// is stored in BBQ_GRANT_W. If there is no grant active, or if the
    /// grant is currently "taken" by the `do_write()` function, the value
    /// is meaningless.
    offset: AtomicUsize,
}

impl UnsafeGrantW {
    const fn new() -> Self {
        Self {
            uc_mu_fgw: UnsafeCell::new(MaybeUninit::uninit()),
            offset: AtomicUsize::new(0),
        }
    }

    // TODO: Could be made safe if we ensure the reference is only taken
    // once. For now, leave unsafe.
    //
    /// This function STORES
    /// MUST be done in a critical section.
    unsafe fn put(&self, grant: GrantW<'static, BUF_SIZE>, offset: usize) -> Result<(), Error> {
        // Note: This also catches the "already latched" state check
        latch_assert_eq(
            logstate::INIT_NO_STORED_GRANT,
            BBQ_STATE.load(Ordering::Relaxed),
        )?;

        self.uc_mu_fgw.get().write(MaybeUninit::new(grant));
        self.offset.store(offset, Ordering::Relaxed);
        BBQ_STATE.store(logstate::INIT_GRANT_IS_STORED, Ordering::Relaxed);
        Ok(())
    }

    // The take function will attempt to provide us with a grant. This grant could
    // come from an existing stored grant (if we are in the INIT_GRANT_IS_STORED
    // state), or from a new grant (if we are in the INIT_NO_STORED_GRANT state).
    //
    // This call to `take()` may fail if we have no space available remaining in the
    // queue, or if we have encountered some kind of latching fault.
    unsafe fn take(&self) -> Result<Option<(GrantW<'static, BUF_SIZE>, usize)>, Error> {
        check_latch(Ordering::Relaxed)?;

        Ok(match BBQ_STATE.load(Ordering::Relaxed) {
            // We have a stored grant. Take it out of the global, and return it to the user
            logstate::INIT_GRANT_IS_STORED => {
                // NOTE: UnsafeCell and MaybeUninit are #[repr(Transparent)], so this
                // cast is acceptable
                let grant = self
                    .uc_mu_fgw
                    .get()
                    .cast::<GrantW<'static, BUF_SIZE>>()
                    .read();

                BBQ_STATE.store(logstate::INIT_NO_STORED_GRANT, Ordering::Relaxed);

                Some((grant, self.offset.load(Ordering::Relaxed)))
            }

            // We *don't* have a stored grant. Attempt to retrieve a new one, and return
            // that to the user, without storing it in the global.
            logstate::INIT_NO_STORED_GRANT => {
                let producer = BBQ_PRODUCER.get_mut()?;

                // We have a new grant, reset the current grant offset back to zero
                self.offset.store(0, Ordering::Relaxed);
                producer.grant_max_remaining(BUF_SIZE).ok().map(|g| (g, 0))
            }

            // We're in a bad place. Store a latching fault, and move on.
            n => {
                // If we aren't already in a latching fault of some kind, set one
                if n < logstate::LATCH_INTERNAL_ERROR {
                    BBQ_STATE.store(logstate::LATCH_INTERNAL_ERROR, Ordering::Relaxed);
                }

                return Err(Error::InternalLatchingFault);
            }
        })
    }
}

unsafe impl Sync for UnsafeGrantW {}

// ----------------------------------------------------------------------------
// Globals
// ----------------------------------------------------------------------------

// The underlying byte storage containing the logs. Always valid
static BBQ: BBBuffer<BUF_SIZE> = BBBuffer::new();

// A tracking variable for ensuring state. Always valid.
static BBQ_STATE: AtomicU8 = AtomicU8::new(logstate::UNINIT);

// The producer half of the logging queue. This field is ONLY
// valid if `init()` has been called.
static BBQ_PRODUCER: UnsafeProducer = UnsafeProducer::new();

// An active write grant to a portion of the `BBQ`, obtained through
// the `BBQ_PRODUCER`. This field is ONLY valid if we are in the
// `INIT_GRANT` state.
static BBQ_GRANT_W: UnsafeGrantW = UnsafeGrantW::new();

// ----------------------------------------------------------------------------
// defmt::Logger interface
//
// This is the implementation of the defmt::Logger interace
// ----------------------------------------------------------------------------

pub(crate) fn should_bail() -> bool {
    let state = BBQ_STATE.load(Ordering::Relaxed);

    let bail = match state {
        // Fast case: all good.
        logstate::INIT_NO_STORED_GRANT => false,

        // We tried to use before initialization. Regardless of the taken state,
        // this is an error. We *might* be able to recover from this in the future,
        // but it is more complicated. For now, just latch the error and signal
        // the user
        logstate::UNINIT => {
            BBQ_STATE.store(logstate::LATCH_USE_BEFORE_INIT, Ordering::Relaxed);
            true
        }

        // Either the taken flag is already set, or we are in an unexpected state
        // on acquisition. Either way, refuse to move forward.
        _ => {
            BBQ_STATE.store(logstate::LATCH_INTERNAL_ERROR, Ordering::Relaxed);
            true
        }
    };

    bail
}

pub(crate) unsafe fn commit_w_grant() {
    // If a grant is active, take it and commit it
    match BBQ_GRANT_W.take() {
        Ok(Some((grant, offset))) => grant.commit(offset),

        // If we have no grant, or an internal error, keep going. We don't
        // want to early return, as that would prevent us from re-enabling
        // interrupts
        _ => {}
    }
}

// ----------------------------------------------------------------------------
// do_write() - This is the main engine of loading bytes into the defmt queue,
// as requested by the defmt::Logger interface.
// ----------------------------------------------------------------------------

// Drain as many bytes to the queue as possible. If the queue is filled,
// then any remaining bytes will be discarded.
pub(crate) fn do_write(mut remaining: &[u8]) {
    while !remaining.is_empty() {
        // The take function will attempt to provide us with a grant. This grant could
        // come from an existing stored grant (if we are in the INIT_GRANT_IS_STORED
        // state), or from a new grant (if we are in the INIT_NO_STORED_GRANT state).
        //
        // This call to `take()` may fail if we have no space available remaining in the
        // queue, or if we have encountered some kind of latching fault.
        match unsafe { BBQ_GRANT_W.take() } {
            // We currently have a grant that is stored. Write as many bytes as possible
            // into this grant
            Ok(Some((mut grant, mut offset))) => {
                let glen = grant.len();

                let min = remaining.len().min(grant.len() - offset);
                grant[offset..][..min].copy_from_slice(&remaining[..min]);
                offset += min;

                remaining = &remaining[min..];

                if offset >= glen {
                    // We have filled the current grant with the requested bytes. Commit the
                    // grant, in order to allow us to potentially get the next grant.
                    //
                    // This leaves the state as `INIT_NO_STORED_GRANT`.
                    grant.commit(offset);
                } else {
                    // We have loaded all bytes into the grant, but we haven't hit the end of
                    // the grant. Store the grant back into the global storage, so we can continue
                    // to re-use it until the `release()` function is called.
                    //
                    // This leaves the state as `INIT_GRANT_IS_STORED`, unless the `put()` fails
                    // (which would set some kind of latching fault).
                    unsafe {
                        // If the put failed, return early
                        if BBQ_GRANT_W.put(grant, offset).is_err() {
                            return;
                        }
                    }
                }
            }

            // No grant available, just return. Bytes are dropped
            Ok(None) => return,

            // A latching fault is active. just return.
            Err(_) => return,
        }
    }
}
