//! Win32 boundary: named file mapping, named mutex, session id, elevation
//! check and a `steady_clock`-compatible clock. The only module in this crate
//! allowed to use `unsafe`.
#![allow(unsafe_code)]

use std::io;
pub(crate) use std::os::windows::io::OwnedHandle;
use std::os::windows::io::{AsRawHandle, FromRawHandle};

use crate::wire::{ActiveConsumers, SharedData};

use windows_sys::Win32::Foundation::{
    CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_ALREADY_EXISTS, GetLastError,
    HANDLE, INVALID_HANDLE_VALUE, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Cryptography::{
    BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
};
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_WRITE, FlushViewOfFile, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
    PAGE_READWRITE, UnmapViewOfFile,
};
use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, GetCurrentProcessId, INFINITE, OpenProcess, OpenProcessToken,
    PROCESS_DUP_HANDLE, ReleaseMutex, WaitForSingleObject,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// A mapped view of a named, pagefile-backed file mapping.
pub(crate) struct Mapping {
    handle: HANDLE,
    view: *mut u8,
    size: usize,
}

impl Mapping {
    /// Opens or creates the mapping; returns whether this call created it.
    pub(crate) fn open(name: &str, size: usize) -> io::Result<(Self, bool)> {
        let name = wide(name);
        // SAFETY: `name` is a NUL-terminated UTF-16 buffer that outlives the call; a null
        // security descriptor is permitted.
        let handle = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                std::ptr::null(),
                PAGE_READWRITE,
                0,
                size as u32,
                name.as_ptr(),
            )
        };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: trivially safe; must be read before any other Win32 call.
        let created = unsafe { GetLastError() } != ERROR_ALREADY_EXISTS;
        // SAFETY: `handle` is a valid mapping handle opened above.
        let view = unsafe { MapViewOfFile(handle, FILE_MAP_WRITE, 0, 0, size) };
        if view.Value.is_null() {
            let err = io::Error::last_os_error();
            // SAFETY: `handle` is valid and owned here; not used afterward.
            unsafe { CloseHandle(handle) };
            return Err(err);
        }
        Ok((
            Self {
                handle,
                view: view.Value.cast(),
                size,
            },
            created,
        ))
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used by the Reader in a later task")
    )]
    pub(crate) fn as_ptr(&self) -> *mut u8 {
        self.view
    }

    /// Test-only: a copy of every byte of the view.
    #[cfg(test)]
    pub(crate) fn bytes(&self) -> Vec<u8> {
        // SAFETY: the view is `size` readable bytes; pagefile-backed pages are initialized.
        unsafe { std::slice::from_raw_parts(self.view, self.size) }.to_vec()
    }

    /// Test-only: sets every byte of the view from `offset` on to `byte`.
    #[cfg(test)]
    pub(crate) fn fill_from(&self, offset: usize, byte: u8) {
        assert!(offset <= self.size);
        // SAFETY: the view is `size` writable bytes, and `offset <= size` (asserted).
        unsafe { self.view.add(offset).write_bytes(byte, self.size - offset) }
    }

    /// Copies the `SharedData` header out of the view. Callers normally hold the SHM mutex, but
    /// some read unlocked, as C++ does; a concurrent writer may then tear the copy.
    ///
    /// # Panics
    /// If the mapping is smaller than `SharedData`.
    pub(crate) fn read_header(&self) -> SharedData {
        assert!(self.size >= size_of::<SharedData>());
        // SAFETY: the view is page-aligned and at least `size_of::<SharedData>()` bytes
        // (asserted above). `SharedData` is plain old data (integers, floats and `CBool`
        // bytes), valid for every bit pattern, so a peer's writes cannot make this read UB. An
        // unlocked read may observe a torn copy, exactly as the unlocked C++ read may; that is
        // not a validity problem.
        unsafe { self.view.cast::<SharedData>().read() }
    }

    /// Copies `header` into the view. Same contract as [`Self::read_header`].
    pub(crate) fn write_header(&self, header: &SharedData) {
        assert!(self.size >= size_of::<SharedData>());
        // SAFETY: as in `read_header`; the view was opened writable.
        unsafe { self.view.cast::<SharedData>().write(*header) }
    }

    /// Copies the `ActiveConsumers` struct out of the view (no lock, as in C++).
    ///
    /// # Panics
    /// If the mapping is smaller than `ActiveConsumers`.
    pub(crate) fn read_active_consumers(&self) -> ActiveConsumers {
        assert!(self.size >= size_of::<ActiveConsumers>());
        // SAFETY: the view is page-aligned and large enough (asserted above); `ActiveConsumers`
        // is integers only, so valid for every bit pattern a peer may write. A concurrent
        // writer may tear the copy, exactly as the unlocked C++ `*p` copy may.
        unsafe { self.view.cast::<ActiveConsumers>().read() }
    }

    /// Writes `value` at byte `offset` of the view without touching neighboring bytes, so
    /// unlocked peers writing other fields (as the C++ `ActiveConsumers` does) are not
    /// clobbered.
    ///
    /// # Panics
    /// If the write would be out of bounds or misaligned.
    pub(crate) fn write_at<T: Scalar>(&self, offset: usize, value: T) {
        let end = offset.checked_add(size_of::<T>());
        assert!(
            end.is_some_and(|end| end <= self.size),
            "write out of bounds"
        );
        assert!(offset.is_multiple_of(align_of::<T>()), "misaligned write");
        // SAFETY: in bounds and aligned (asserted; the view itself is page-aligned); the view
        // is writable; `T: Scalar` is a padding-free integer, so every written byte is
        // initialized; no Rust reference into the view exists, so no aliasing is violated.
        unsafe { self.view.add(offset).cast::<T>().write(value) }
    }

    /// Zeroes the `ActiveConsumers` byte range (C++ `*mView = {}`; the default is all zeros).
    ///
    /// # Panics
    /// If the mapping is smaller than `ActiveConsumers`.
    pub(crate) fn zero_active_consumers(&self) {
        assert!(self.size >= size_of::<ActiveConsumers>());
        // SAFETY: in bounds (asserted) and writable. `write_bytes` sets every byte, padding
        // included, to 0, so no uninitialized byte reaches the view.
        unsafe { self.view.write_bytes(0, size_of::<ActiveConsumers>()) }
    }

    /// `FlushViewOfFile` over the whole header, as the C++ `Writer::Detach` does.
    pub(crate) fn flush(&self) {
        // SAFETY: `view` is a live mapped view of at least `size_of::<SharedData>()` bytes.
        unsafe { FlushViewOfFile(self.view.cast(), size_of::<SharedData>()) };
    }
}

mod sealed {
    pub trait Sealed {}
}

/// Padding-free integer types that [`Mapping::write_at`] may write.
pub(crate) trait Scalar: Copy + sealed::Sealed {}
impl sealed::Sealed for u32 {}
impl sealed::Sealed for u64 {}
impl sealed::Sealed for i64 {}
impl Scalar for u32 {}
impl Scalar for u64 {}
impl Scalar for i64 {}

// SAFETY: the mapping handle and view address are process-wide (no thread affinity), and
// every accessor copies data in or out through raw pointers without handing out references,
// so moving a `Mapping` to another thread (e.g. into the process-wide ActiveConsumers cache)
// is sound.
unsafe impl Send for Mapping {}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: `view` and `handle` were obtained in `open` and are released exactly once.
        unsafe {
            UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                Value: self.view.cast(),
            });
            CloseHandle(self.handle);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LockOutcome {
    Acquired,
    /// The previous owner exited without unlocking; the caller must reset shared state.
    Abandoned,
}

/// A named mutex. Lock and unlock must happen on the same thread.
pub(crate) struct NamedMutex(HANDLE);

impl NamedMutex {
    pub(crate) fn open(name: &str) -> io::Result<Self> {
        let name = wide(name);
        // SAFETY: `name` is NUL-terminated and outlives the call; null attributes are allowed.
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(handle))
    }

    fn wait(&self, ms: u32) -> io::Result<Option<LockOutcome>> {
        // SAFETY: `self.0` is a valid mutex handle for the lifetime of `self`.
        match unsafe { WaitForSingleObject(self.0, ms) } {
            WAIT_OBJECT_0 => Ok(Some(LockOutcome::Acquired)),
            WAIT_ABANDONED => Ok(Some(LockOutcome::Abandoned)),
            WAIT_TIMEOUT => Ok(None),
            _ => Err(io::Error::last_os_error()),
        }
    }

    pub(crate) fn lock(&self) -> io::Result<LockOutcome> {
        self.wait(INFINITE)?
            .ok_or_else(|| io::Error::other("infinite wait timed out"))
    }

    pub(crate) fn try_lock(&self) -> io::Result<Option<LockOutcome>> {
        self.wait(0)
    }

    pub(crate) fn unlock(&self) {
        // SAFETY: `self.0` is a valid mutex handle. Failure (not the owner) is a caller bug
        // with no recovery, matching the C++ which ignores the result.
        unsafe { ReleaseMutex(self.0) };
    }
}

impl Drop for NamedMutex {
    fn drop(&mut self) {
        // SAFETY: handle is valid and closed exactly once.
        unsafe { CloseHandle(self.0) };
    }
}

pub(crate) fn session_id() -> u64 {
    let mut r = [0u8; 4];
    // SAFETY: the buffer is valid for 4 bytes; a null algorithm handle is allowed with
    // BCRYPT_USE_SYSTEM_PREFERRED_RNG.
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            r.as_mut_ptr(),
            4,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    assert!(status >= 0, "BCryptGenRandom failed: {status:#x}");
    // SAFETY: trivially safe.
    let pid = unsafe { GetCurrentProcessId() };
    (u64::from(pid) << 32) | u64::from(u32::from_ne_bytes(r))
}

pub(crate) fn is_elevated() -> bool {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: the pseudo-handle is always valid; `token` is a valid out pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return false;
    }
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut len = 0u32;
    // SAFETY: `token` is valid; the buffer is a TOKEN_ELEVATION of the stated size.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            (&raw mut elevation).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
    };
    // SAFETY: `token` was opened above and is closed once.
    unsafe { CloseHandle(token) };
    ok != 0 && elevation.TokenIsElevated != 0
}

fn qpc_to_ns(qpc: i64, freq: i64) -> i64 {
    (qpc / freq) * 1_000_000_000 + (qpc % freq) * 1_000_000_000 / freq
}

fn qpc() -> (i64, i64) {
    let (mut c, mut f) = (0i64, 0i64);
    // SAFETY: both out pointers are valid; these calls cannot fail on supported Windows.
    unsafe {
        QueryPerformanceCounter(&mut c);
        QueryPerformanceFrequency(&mut f);
    }
    (c, f)
}

/// Matches MSVC `std::chrono::steady_clock::now()` in nanoseconds.
pub(crate) fn steady_clock_now_ns() -> i64 {
    let (c, f) = qpc();
    qpc_to_ns(c, f)
}

/// `OpenProcess(PROCESS_DUP_HANDLE, FALSE, pid)`; `None` on failure (e.g. pid 0 or exited).
pub(crate) fn open_process_for_dup(pid: u32) -> Option<OwnedHandle> {
    // SAFETY: plain call; the result is checked before use.
    let h = unsafe { OpenProcess(PROCESS_DUP_HANDLE, 0, pid) };
    // SAFETY: `h` is a fresh, owned, non-null process handle.
    (!h.is_null()).then(|| unsafe { OwnedHandle::from_raw_handle(h) })
}

/// `DuplicateHandle(from, handle, GetCurrentProcess(), ..., DUPLICATE_SAME_ACCESS)`.
pub(crate) fn duplicate_handle(from: &OwnedHandle, handle: u64) -> io::Result<OwnedHandle> {
    let mut out: HANDLE = std::ptr::null_mut();
    // SAFETY: `from` is a live process handle with PROCESS_DUP_HANDLE; `handle` is a value in
    // that process's handle table, which the kernel validates; `out` is a valid out pointer.
    let ok = unsafe {
        DuplicateHandle(
            from.as_raw_handle(),
            handle as usize as HANDLE,
            GetCurrentProcess(),
            &mut out,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: on success `out` is a new handle owned by this process.
    Ok(unsafe { OwnedHandle::from_raw_handle(out) })
}

/// The raw value of `h`, as stored in the wire `FixedSizeHandle`.
pub(crate) fn raw(h: &OwnedHandle) -> u64 {
    h.as_raw_handle() as usize as u64
}

#[cfg(test)]
pub(crate) fn create_event() -> OwnedHandle {
    // SAFETY: null attributes and name are allowed.
    let h = unsafe {
        windows_sys::Win32::System::Threading::CreateEventW(
            std::ptr::null(),
            0,
            0,
            std::ptr::null(),
        )
    };
    assert!(!h.is_null(), "{}", io::Error::last_os_error());
    // SAFETY: fresh owned handle.
    unsafe { OwnedHandle::from_raw_handle(h) }
}

#[cfg(test)]
pub(crate) fn same_object(a: u64, b: u64) -> bool {
    // SAFETY: plain call on handle values; invalid values just compare unequal.
    unsafe {
        windows_sys::Win32::Foundation::CompareObjectHandles(
            a as usize as HANDLE,
            b as usize as HANDLE,
        ) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique(test: &str) -> String {
        format!("okb-test.{test}.{}/x", std::process::id())
    }

    #[test]
    fn mapping_is_shared_between_opens() {
        let name = unique("mapping");
        let (a, created_a) = Mapping::open(&name, 64).unwrap();
        let (b, created_b) = Mapping::open(&name, 64).unwrap();
        assert!(created_a);
        assert!(!created_b);
        // SAFETY: both views are 64 bytes and alias the same pages; single-threaded test.
        unsafe {
            a.as_ptr().write(0xAB);
            assert_eq!(b.as_ptr().read(), 0xAB);
        }
    }

    #[test]
    fn abandoned_mutex_resets_header() {
        let name = unique("mutex");
        let m = NamedMutex::open(&name).unwrap();
        let n2 = name.clone();
        std::thread::spawn(move || {
            let t = NamedMutex::open(&n2).unwrap();
            assert_eq!(t.lock().unwrap(), LockOutcome::Acquired);
            // exit without unlock
        })
        .join()
        .unwrap();
        assert_eq!(m.lock().unwrap(), LockOutcome::Abandoned);
        m.unlock();
        assert_eq!(m.try_lock().unwrap(), Some(LockOutcome::Acquired));
        m.unlock();
    }

    #[test]
    fn qpc_to_ns_fixed_inputs() {
        assert_eq!(
            qpc_to_ns(1_234_567_890_123, 10_000_000),
            123_456_789_012_300
        );
        // qpc * 1e9 would overflow i64 here.
        assert_eq!(
            qpc_to_ns(100_000_000_000_000, 3_579_545),
            27_936_511_484_001_458
        );
    }

    #[test]
    fn steady_clock_matches_cpp_formula() {
        let a = steady_clock_now_ns();
        let b = steady_clock_now_ns();
        assert!(b >= a);
        let (c, f) = qpc();
        assert!((qpc_to_ns(c, f) - b).abs() < 1_000_000_000);
    }
}
