use core::cell::RefCell;
use core::ops::{Deref, DerefMut};
use embassy_futures::select::{select4, Either4};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::mutex::{Mutex as AsyncMutex, MutexGuard};

static SCRATCHPAD: BlockingMutex<CriticalSectionRawMutex, RefCell<[u8; 4096]>> =
    BlockingMutex::new(RefCell::new([0u8; 4096]));

/// Execute a synchronous compute-only operation using the 4 KB scratchpad buffer.
/// Non-yieldable: closure `f` cannot `.await`.
pub fn with_scratchpad<R>(f: impl FnOnce(&mut [u8]) -> R) -> R {
    SCRATCHPAD.lock(|cell| {
        let mut buf = cell.borrow_mut();
        f(&mut *buf)
    })
}

/// Serialize a value to JSON into the scratchpad buffer zero-heap and pass the `&str` to closure `f`.
pub fn serialize_to_scratchpad<T: serde::Serialize, R>(
    value: &T,
    f: impl FnOnce(&str) -> R,
) -> Result<R, ()> {
    with_scratchpad(|buf| {
        let len = serde_json_core::to_slice(value, buf).map_err(|_| ())?;
        let str_slice = core::str::from_utf8(&buf[..len]).map_err(|_| ())?;
        Ok(f(str_slice))
    })
}

pub struct NetBufferLease<'a, const SIZE: usize> {
    guard: MutexGuard<'a, CriticalSectionRawMutex, [u8; SIZE]>,
}

impl<'a, const SIZE: usize> Deref for NetBufferLease<'a, SIZE> {
    type Target = [u8; SIZE];
    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl<'a, const SIZE: usize> DerefMut for NetBufferLease<'a, SIZE> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

pub struct NetBufferPool<const SIZE: usize> {
    buffers: [AsyncMutex<CriticalSectionRawMutex, [u8; SIZE]>; 4],
}

impl<const SIZE: usize> NetBufferPool<SIZE> {
    pub const fn new() -> Self {
        Self {
            buffers: [
                AsyncMutex::new([0u8; SIZE]),
                AsyncMutex::new([0u8; SIZE]),
                AsyncMutex::new([0u8; SIZE]),
                AsyncMutex::new([0u8; SIZE]),
            ],
        }
    }

    pub async fn acquire(&self) -> NetBufferLease<'_, SIZE> {
        for buf in &self.buffers {
            if let Ok(guard) = buf.try_lock() {
                return NetBufferLease { guard };
            }
        }

        let guard = match select4(
            self.buffers[0].lock(),
            self.buffers[1].lock(),
            self.buffers[2].lock(),
            self.buffers[3].lock(),
        )
        .await
        {
            Either4::First(g) => g,
            Either4::Second(g) => g,
            Either4::Third(g) => g,
            Either4::Fourth(g) => g,
        };
        NetBufferLease { guard }
    }
}

pub static NET_BUFFER_POOL: NetBufferPool<4096> = NetBufferPool::new();
