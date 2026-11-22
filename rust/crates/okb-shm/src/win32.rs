//! Win32 boundary: named file mapping, named mutex, session id, elevation
//! check and a `steady_clock`-compatible clock. The only module in this crate
//! allowed to use `unsafe`.
#![allow(unsafe_code)]

use std::io;

use crate::wire::SharedData;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, INVALID_HANDLE_VALUE, WAIT_ABANDONED,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
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
    CreateMutexW, GetCurrentProcess, GetCurrentProcessId, INFINITE, OpenProcessToken, ReleaseMutex,
    WaitForSingleObject,
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

    /// Copies the `SharedData` header out of the view. The caller must hold the SHM mutex (or
    /// be the sole user).
    ///
    /// # Panics
    /// If the mapping is smaller than `SharedData`.
    pub(crate) fn read_header(&self) -> SharedData {
        assert!(self.size >= size_of::<SharedData>());
        // SAFETY: the view is page-aligned and at least `size_of::<SharedData>()` bytes
        // (asserted above). `SharedData` is plain old data (integers, floats and `CBool`
        // bytes), valid for every bit pattern, so a peer's writes cannot make this read UB;
        // cross-process access is serialized by the SHM mutex.
        unsafe { self.view.cast::<SharedData>().read() }
    }

    /// Copies `header` into the view. Same contract as [`Self::read_header`].
    pub(crate) fn write_header(&self, header: &SharedData) {
        assert!(self.size >= size_of::<SharedData>());
        // SAFETY: as in `read_header`; the view was opened writable.
        unsafe { self.view.cast::<SharedData>().write(*header) }
    }

    /// `FlushViewOfFile` over the whole header, as the C++ `Writer::Detach` does.
    pub(crate) fn flush(&self) {
        // SAFETY: `view` is a live mapped view of at least `size_of::<SharedData>()` bytes.
        unsafe { FlushViewOfFile(self.view.cast(), size_of::<SharedData>()) };
    }
}

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

#[expect(dead_code, reason = "used by the Reader in a later task")]
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

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by the Reader in a later task")
)]
fn qpc_to_ns(qpc: i64, freq: i64) -> i64 {
    (qpc / freq) * 1_000_000_000 + (qpc % freq) * 1_000_000_000 / freq
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by the Reader in a later task")
)]
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
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by the Reader in a later task")
)]
pub(crate) fn steady_clock_now_ns() -> i64 {
    let (c, f) = qpc();
    qpc_to_ns(c, f)
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
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used by the Reader in a later task")
    )]
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
