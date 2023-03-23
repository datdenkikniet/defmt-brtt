use core::{
    ptr,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::consts::BUF_SIZE;

#[repr(C)]
struct Header {
    id: [u8; 16],
    max_up_channels: usize,
    max_down_channels: usize,
    up_channel: Channel,
}

const MODE_MASK: usize = 0b11;
/// Block the application if the RTT buffer is full, wait for the host to read data.
const MODE_BLOCK_IF_FULL: usize = 2;
/// Don't block if the RTT buffer is full. Truncate data to output as much as fits.
const MODE_NON_BLOCKING_TRIM: usize = 1;

// make sure we only get shared references to the header/channel (avoid UB)
/// # Safety
/// `Channel` API is not re-entrant; this handle should not be held from different execution
/// contexts (e.g. thread-mode, interrupt context)
pub(crate) unsafe fn handle() -> &'static Channel {
    // NOTE the `rtt-target` API is too permissive. It allows writing arbitrary data to any
    // channel (`set_print_channel` + `rprint*`) and that can corrupt defmt log frames.
    // So we declare the RTT control block here and make it impossible to use `rtt-target` together
    // with this crate.
    #[no_mangle]
    static mut _SEGGER_RTT: Header = Header {
        id: *b"SEGGER RTT\0\0\0\0\0\0",
        max_up_channels: 1,
        max_down_channels: 0,
        up_channel: Channel {
            name: &NAME as *const _ as *const u8,
            buffer: unsafe { &mut BUFFER as *mut _ as *mut u8 },
            size: BUF_SIZE,
            write: AtomicUsize::new(0),
            read: AtomicUsize::new(0),
            flags: AtomicUsize::new(MODE_NON_BLOCKING_TRIM),
        },
    };

    #[cfg_attr(target_os = "macos", link_section = ".uninit,defmt-rtt.BUFFER")]
    #[cfg_attr(not(target_os = "macos"), link_section = ".uninit.defmt-rtt.BUFFER")]
    static mut BUFFER: [u8; BUF_SIZE] = [0; BUF_SIZE];

    // Place NAME in data section, so the whole RTT header can be read from RAM.
    // This is useful if flash access gets disabled by the firmware at runtime.
    #[link_section = ".data"]
    static NAME: [u8; 6] = *b"defmt\0";

    &_SEGGER_RTT.up_channel
}

pub(crate) fn do_write(bytes: &[u8]) {
    unsafe { handle().write_all(bytes) }
}

/// RTT Up channel
#[repr(C)]
pub(crate) struct Channel {
    pub name: *const u8,
    /// Pointer to the RTT buffer.
    pub buffer: *mut u8,
    pub size: usize,
    /// Written by the target.
    pub write: AtomicUsize,
    /// Written by the host.
    pub read: AtomicUsize,
    /// Channel properties.
    ///
    /// Currently, only the lowest 2 bits are used to set the channel mode (see constants below).
    pub flags: AtomicUsize,
}

impl Channel {
    pub fn write_all(&self, mut bytes: &[u8]) {
        // the host-connection-status is only modified after RAM initialization while the device is
        // halted, so we only need to check it once before the write-loop
        let write = match self.host_is_connected() {
            true => Self::blocking_write,
            false => Self::nonblocking_write,
        };

        while !bytes.is_empty() {
            let consumed = write(self, bytes);
            if consumed != 0 {
                bytes = &bytes[consumed..];
            }
        }
    }

    fn blocking_write(&self, bytes: &[u8]) -> usize {
        if bytes.is_empty() {
            return 0;
        }

        // calculate how much space is left in the buffer
        let read = self.read.load(Ordering::Relaxed);
        let write = self.write.load(Ordering::Acquire);
        let available = available_buffer_size(read, write);

        // abort if buffer is full
        if available == 0 {
            return 0;
        }

        self.write_impl(bytes, write, available)
    }

    fn nonblocking_write(&self, bytes: &[u8]) -> usize {
        let write = self.write.load(Ordering::Acquire);

        // NOTE truncate at BUF_SIZE to avoid more than one "wrap-around" in a single `write` call
        self.write_impl(bytes, write, BUF_SIZE)
    }

    fn write_impl(&self, bytes: &[u8], cursor: usize, available: usize) -> usize {
        let len = bytes.len().min(available);

        // copy `bytes[..len]` to the RTT buffer
        unsafe {
            if cursor + len > BUF_SIZE {
                // split memcpy
                let pivot = BUF_SIZE - cursor;
                ptr::copy_nonoverlapping(bytes.as_ptr(), self.buffer.add(cursor), pivot);
                ptr::copy_nonoverlapping(bytes.as_ptr().add(pivot), self.buffer, len - pivot);
            } else {
                // single memcpy
                ptr::copy_nonoverlapping(bytes.as_ptr(), self.buffer.add(cursor), len);
            }
        }

        // adjust the write pointer, so the host knows that there is new data
        self.write
            .store(cursor.wrapping_add(len) % BUF_SIZE, Ordering::Release);

        // return the number of bytes written
        len
    }

    pub fn flush(&self) {
        // return early, if host is disconnected
        if !self.host_is_connected() {
            return;
        }

        // busy wait, until the read- catches up with the write-pointer
        let read = || self.read.load(Ordering::Relaxed);
        let write = || self.write.load(Ordering::Relaxed);
        while read() != write() {}
    }

    fn host_is_connected(&self) -> bool {
        // we assume that a host is connected if we are in blocking-mode. this is what probe-run does.
        self.flags.load(Ordering::Relaxed) & MODE_MASK == MODE_BLOCK_IF_FULL
    }
}

/// How much space is left in the buffer?
fn available_buffer_size(read_cursor: usize, write_cursor: usize) -> usize {
    if read_cursor > write_cursor {
        read_cursor - write_cursor - 1
    } else if read_cursor == 0 {
        BUF_SIZE - write_cursor - 1
    } else {
        BUF_SIZE - write_cursor
    }
}
