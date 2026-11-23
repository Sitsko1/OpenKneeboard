//! C ABI over `okb-shm` and `okb-vr`, for the C++ test harness.
//!
//! Handles are opaque and `!Send`: use each one only on the thread that created it. Every
//! entry point aborts the process if the Rust side panics (C++ `fatal()` parity); no unwind
//! crosses the ABI. Null handles are a caller bug and also abort, except in the `_free`
//! functions, which accept null.
#![allow(unsafe_code)]

use std::ffi::{CStr, c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};

use okb_shm::wire::{Config, LayerConfig, PixelRect, PixelSize};
use okb_shm::{ConsumerKind, FrameError, Names, ReadError, Reader, Version};
use okb_vr::{ActiveViewUpdate, Pose, Quat, SpriteAtlas, Vec2, Vec3, VrKneeboard};

/// Capacity of `OkbFrame::layers`. A literal, as cbindgen does not export constants from
/// dependencies.
pub const MAX_VIEW_COUNT: usize = 16;
const _: () = assert!(MAX_VIEW_COUNT == okb_shm::MAX_VIEW_COUNT);

/// Runs `f`, aborting the process if it panics.
fn ffi<R>(f: impl FnOnce() -> R) -> R {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| std::process::abort())
}

/// # Safety
/// `p` is null or a live pointer from the matching `_new` function, not aliased for `'a`.
unsafe fn handle<'a, T>(p: *mut T) -> &'a mut T {
    // SAFETY: guaranteed by the caller; null is rejected (panic, so abort).
    unsafe { p.as_mut() }.expect("null okb-ffi handle")
}

/// SHM object names (`OkbShmNames`).
pub struct ShmNames(Names);

/// An okb-shm `Writer` plus the lock state of the C++ "Lockable" interface (`OkbShmWriter`).
pub struct ShmWriter {
    /// Borrows `*writer`; always dropped before `writer` is freed.
    guard: Option<okb_shm::WriterGuard<'static>>,
    /// From `Box::into_raw`; only accessed through `guard` while that is `Some`.
    writer: *mut okb_shm::Writer,
}

impl ShmWriter {
    fn guard(&mut self) -> &mut okb_shm::WriterGuard<'static> {
        self.guard.as_mut().expect("okb-ffi writer is not locked")
    }

    fn set_guard(
        &mut self,
        f: impl FnOnce(&'static mut okb_shm::Writer) -> Option<okb_shm::WriterGuard<'static>>,
    ) -> bool {
        assert!(self.guard.is_none(), "okb-ffi writer is already locked");
        // SAFETY: `writer` is live until `Drop`, which drops `guard` first; no other
        // reference to it exists while `guard` is `None`.
        self.guard = f(unsafe { &mut *self.writer });
        self.guard.is_some()
    }
}

impl Drop for ShmWriter {
    fn drop(&mut self) {
        self.guard = None;
        // SAFETY: from `Box::into_raw`; the only borrow (`guard`) is gone.
        drop(unsafe { Box::from_raw(self.writer) });
    }
}

/// An okb-shm `Reader` (`OkbShmReader`).
pub struct ShmReader(Reader<Box<dyn FnMut()>>);

/// C++ `Writer::NextFrameInfo`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NextFrameInfo {
    pub texture_index: u8,
    pub fence_out: i64,
}

/// C++ `SHM::Frame`, with the layers inline.
#[repr(C)]
pub struct Frame {
    pub config: Config,
    pub layers: [LayerConfig; MAX_VIEW_COUNT],
    pub layer_count: u32,
    pub texture: u64,
    pub fence: u64,
    pub fence_in: i64,
    pub index: u8,
}

/// Returns null if `reverse_domain` is null or not UTF-8. Free with `okb_shm_names_free`.
///
/// # Safety
/// `reverse_domain` is null or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn okb_shm_names_new(
    reverse_domain: *const c_char,
    major: u32,
    minor: u32,
    patch: u32,
    build: u32,
) -> *mut ShmNames {
    ffi(|| {
        if reverse_domain.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: non-null and NUL-terminated per the contract.
        let Ok(domain) = unsafe { CStr::from_ptr(reverse_domain) }.to_str() else {
            return std::ptr::null_mut();
        };
        let version = Version {
            major,
            minor,
            patch,
            build,
        };
        Box::into_raw(Box::new(ShmNames(Names::new(domain, version))))
    })
}

/// # Safety
/// `names` is null or from `okb_shm_names_new`, and not used afterward.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn okb_shm_names_free(names: *mut ShmNames) {
    ffi(|| {
        if !names.is_null() {
            // SAFETY: from `Box::into_raw` in `okb_shm_names_new`, freed once.
            drop(unsafe { Box::from_raw(names) });
        }
    })
}

/// Returns null where C++ `Writer::operator bool` is false. Overwrites the SHM header.
///
/// # Safety
/// `names` is a live `OkbShmNames`; it may be freed once this returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn okb_shm_writer_new(
    names: *const ShmNames,
    gpu_luid: u64,
) -> *mut ShmWriter {
    ffi(|| {
        // SAFETY: live per the contract.
        let names = unsafe { names.as_ref() }.expect("null okb-ffi handle");
        match okb_shm::Writer::new(&names.0, gpu_luid) {
            Some(w) => Box::into_raw(Box::new(ShmWriter {
                guard: None,
                writer: Box::into_raw(Box::new(w)),
            })),
            None => std::ptr::null_mut(),
        }
    })
}

/// Releases the lock if held, then detaches (as C++ `~Writer`).
///
/// # Safety
/// `writer` is null or a live `OkbShmWriter` on its creating thread, not used afterward.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn okb_shm_writer_free(writer: *mut ShmWriter) {
    ffi(|| {
        if !writer.is_null() {
            // SAFETY: from `Box::into_raw` in `okb_shm_writer_new`, freed once.
            drop(unsafe { Box::from_raw(writer) });
        }
    })
}

/// Blocks until the SHM mutex is held. Aborts if already locked through this handle.
///
/// # Safety
/// `writer` is a live `OkbShmWriter` on its creating thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn okb_shm_writer_lock(writer: *mut ShmWriter) {
    // SAFETY: forwarded contract.
    ffi(|| unsafe { handle(writer) }.set_guard(|w| Some(w.lock())));
}

/// False if another holder has the SHM mutex. Aborts if already locked through this handle.
///
/// # Safety
/// `writer` is a live `OkbShmWriter` on its creating thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn okb_shm_writer_try_lock(writer: *mut ShmWriter) -> bool {
    // SAFETY: forwarded contract.
    ffi(|| unsafe { handle(writer) }.set_guard(|w| w.try_lock()))
}

/// Aborts if not locked.
///
/// # Safety
/// `writer` is a live `OkbShmWriter` on its creating thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn okb_shm_writer_unlock(writer: *mut ShmWriter) {
    ffi(|| {
        // SAFETY: forwarded contract.
        let w = unsafe { handle(writer) };
        assert!(w.guard.take().is_some(), "okb-ffi writer is not locked");
    })
}

/// Requires the lock (aborts otherwise), as do the writer functions below except
/// `okb_shm_writer_frame_count`.
///
/// # Safety
/// `writer` is a live `OkbShmWriter` on its creating thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn okb_shm_writer_submit_empty_frame(writer: *mut ShmWriter) {
    // SAFETY: forwarded contract.
    ffi(|| unsafe { handle(writer) }.guard().submit_empty_frame());
}

/// # Safety
/// `writer` is a live `OkbShmWriter` on its creating thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn okb_shm_writer_begin_frame(writer: *mut ShmWriter) -> NextFrameInfo {
    ffi(|| {
        // SAFETY: forwarded contract.
        let info = unsafe { handle(writer) }.guard().begin_frame();
        NextFrameInfo {
            texture_index: info.texture_index,
            fence_out: info.fence_out,
        }
    })
}

/// Aborts if `count` exceeds 16 or `info` does not match the last `begin_frame`.
///
/// # Safety
/// `writer` is a live `OkbShmWriter` on its creating thread; `config` is a valid
/// `OkbConfig`; `layers` points to `count` `OkbLayerConfig`s (may be null if `count` is 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn okb_shm_writer_submit_frame(
    writer: *mut ShmWriter,
    info: NextFrameInfo,
    config: *const Config,
    layers: *const LayerConfig,
    count: usize,
    texture: u64,
    fence: u64,
) {
    ffi(|| {
        // SAFETY: valid per the contract.
        let config = unsafe { config.as_ref() }.expect("null config");
        assert!(
            count <= MAX_VIEW_COUNT,
            "Asked to publish {count} layers, but max is {MAX_VIEW_COUNT}"
        );
        let layers = if count == 0 {
            &[][..]
        } else {
            assert!(!layers.is_null(), "null layers");
            // SAFETY: `layers` points to `count` initialized `LayerConfig`s per the contract;
            // every bit pattern is valid for them (`CBool` for C++ `bool`).
            unsafe { std::slice::from_raw_parts(layers, count) }
        };
        let info = okb_shm::NextFrameInfo {
            texture_index: info.texture_index,
            fence_out: info.fence_out,
        };
        // SAFETY: forwarded contract.
        unsafe { handle(writer) }
            .guard()
            .submit_frame(info, config, layers, texture, fence);
    })
}

/// # Safety
/// `writer` is a live `OkbShmWriter` on its creating thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn okb_shm_writer_detach(writer: *mut ShmWriter) {
    // SAFETY: forwarded contract.
    ffi(|| unsafe { handle(writer) }.guard().detach());
}

/// C++ `Writer::GetFrameCountForMetricsOnly`; works locked or unlocked.
///
/// # Safety
/// `writer` is a live `OkbShmWriter` on its creating thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn okb_shm_writer_frame_count(writer: *mut ShmWriter) -> u64 {
    ffi(|| {
        // SAFETY: forwarded contract.
        let w = unsafe { handle(writer) };
        match &w.guard {
            Some(g) => g.frame_count(),
            // SAFETY: unlocked, so nothing else borrows `*writer`; live until `Drop`.
            None => unsafe { &*w.writer }.frame_count(),
        }
    })
}

/// `kind` is a C++ `SHM::ConsumerKind` value. Returns null if `kind` is unknown or the SHM is
/// unavailable. `on_session_changed(ctx)` (nullable) runs on the reader's thread inside
/// `okb_shm_reader_maybe_get`, with the SHM mutex held; it must not unwind, throw, or call
/// back into this reader.
///
/// # Safety
/// `names` is a live `OkbShmNames` (it may be freed once this returns); `ctx` stays valid
/// for `on_session_changed` until the reader is freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn okb_shm_reader_new(
    names: *const ShmNames,
    kind: u32,
    gpu_luid: u64,
    on_session_changed: Option<unsafe extern "C" fn(*mut c_void)>,
    ctx: *mut c_void,
) -> *mut ShmReader {
    ffi(|| {
        // SAFETY: live per the contract.
        let names = unsafe { names.as_ref() }.expect("null okb-ffi handle");
        let kind = match kind {
            1 => ConsumerKind::OpenVr,
            2 => ConsumerKind::OpenXrD3d11,
            4 => ConsumerKind::OpenXrD3d12,
            8 => ConsumerKind::OpenXrVulkan2,
            u32::MAX => ConsumerKind::Viewer,
            _ => return std::ptr::null_mut(),
        };
        let callback: Box<dyn FnMut()> = Box::new(move || {
            if let Some(f) = on_session_changed {
                // SAFETY: the caller keeps `ctx` valid for `f` until the reader is freed.
                unsafe { f(ctx) }
            }
        });
        match Reader::new(&names.0, kind, gpu_luid, callback) {
            Some(r) => Box::into_raw(Box::new(ShmReader(r))),
            None => std::ptr::null_mut(),
        }
    })
}

/// Closes every handle the reader returned.
///
/// # Safety
/// `reader` is null or a live `OkbShmReader` on its creating thread, not used afterward.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn okb_shm_reader_free(reader: *mut ShmReader) {
    ffi(|| {
        if !reader.is_null() {
            // SAFETY: from `Box::into_raw` in `okb_shm_reader_new`, freed once.
            drop(unsafe { Box::from_raw(reader) });
        }
    })
}

/// C++ `Reader::operator bool`.
///
/// # Safety
/// `reader` is a live `OkbShmReader` on its creating thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn okb_shm_reader_has_feeder(reader: *mut ShmReader) -> bool {
    // SAFETY: forwarded contract.
    ffi(|| unsafe { handle(reader) }.0.has_feeder())
}

/// C++ `Reader::MaybeGet`. Returns 0 and fills `*out` on success; otherwise leaves `*out`
/// unchanged and returns `1 + Frame::Error` (1 `IncorrectGPU`, 2 `NoFeeder`, 3 `NoLayers`,
/// 4 `UnusableHandles`), or `-(Win32 error)` if `DuplicateHandle` failed.
///
/// `out->texture` and `out->fence` are handles owned by the reader: do not close them. They are
/// closed on the next session change, when that swapchain slot's foreign handle changes, or
/// when the reader is freed.
///
/// # Safety
/// `reader` is a live `OkbShmReader` on its creating thread; `out` is valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn okb_shm_reader_maybe_get(reader: *mut ShmReader, out: *mut Frame) -> i32 {
    ffi(|| {
        assert!(!out.is_null(), "null out");
        // SAFETY: forwarded contract.
        match unsafe { handle(reader) }.0.maybe_get() {
            Ok(f) => {
                let mut layers = [LayerConfig::default(); MAX_VIEW_COUNT];
                layers[..f.layers.len()].copy_from_slice(&f.layers);
                let frame = Frame {
                    config: f.config,
                    layers,
                    layer_count: f.layers.len() as u32,
                    texture: f.texture,
                    fence: f.fence,
                    fence_in: f.fence_in,
                    index: f.index,
                };
                // SAFETY: non-null and valid for writes per the contract.
                unsafe { out.write(frame) };
                0
            }
            Err(ReadError::Frame(e)) => match e {
                FrameError::IncorrectGpu => 1,
                FrameError::NoFeeder => 2,
                FrameError::NoLayers => 3,
                FrameError::UnusableHandles => 4,
            },
            Err(ReadError::DuplicateHandle(e)) => -e.raw_os_error().unwrap_or(0).clamp(1, i32::MAX),
        }
    })
}

/// One `okb_vr::VrLayer`: `layer_index` is the layer's index in the input array.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VrLayerOut {
    pub layer_index: u32,
    pub kneeboard_pose: Pose,
    pub kneeboard_size: Vec2,
    pub kneeboard_opacity: f32,
    pub is_looking_at_kneeboard: bool,
}

/// C++ `VRKneeboard`. Free with `okb_vr_kneeboard_free`.
#[unsafe(no_mangle)]
pub extern "C" fn okb_vr_kneeboard_new() -> *mut VrKneeboard {
    ffi(|| Box::into_raw(Box::new(VrKneeboard::new())))
}

/// # Safety
/// `kb` is null or from `okb_vr_kneeboard_new`, and not used afterward.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn okb_vr_kneeboard_free(kb: *mut VrKneeboard) {
    ffi(|| {
        if !kb.is_null() {
            // SAFETY: from `Box::into_raw` in `okb_vr_kneeboard_new`, freed once.
            drop(unsafe { Box::from_raw(kb) });
        }
    })
}

/// C++ `VRKneeboard::GetLayers`. Writes `*out_count` layers to `out`, and to `*active_view`
/// the ID C++ would pass to `SHM::ActiveConsumers::SetActiveInGameViewID` (0 for `{}`), or
/// `UINT64_MAX` if it would not call it. Aborts if `count` exceeds 16.
///
/// # Safety
/// `kb` is a live `OkbVrKneeboard` on its creating thread; `config` is a valid `OkbConfig`;
/// `layers` points to `count` `OkbLayerConfig`s (may be null if `count` is 0); `out` is valid
/// for 16 writes; `out_count` and `active_view` are valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn okb_vr_kneeboard_get_layers(
    kb: *mut VrKneeboard,
    config: *const Config,
    layers: *const LayerConfig,
    count: usize,
    hmd: Pose,
    out: *mut VrLayerOut,
    out_count: *mut u32,
    active_view: *mut u64,
) {
    ffi(|| {
        // SAFETY: valid per the contract.
        let config = unsafe { config.as_ref() }.expect("null config");
        assert!(
            count <= MAX_VIEW_COUNT,
            "Asked for {count} layers, but max is {MAX_VIEW_COUNT}"
        );
        assert!(
            !out.is_null() && !out_count.is_null() && !active_view.is_null(),
            "null out"
        );
        let layers = if count == 0 {
            &[][..]
        } else {
            assert!(!layers.is_null(), "null layers");
            // SAFETY: `layers` points to `count` initialized `LayerConfig`s per the contract;
            // every bit pattern is valid for them (`CBool` for C++ `bool`).
            unsafe { std::slice::from_raw_parts(layers, count) }
        };
        // SAFETY: forwarded contract.
        let (vr_layers, update) = unsafe { handle(kb) }.get_layers(config, layers, hmd);
        let base = layers.as_ptr().addr();
        for (i, layer) in vr_layers.iter().enumerate() {
            let index = (std::ptr::from_ref(layer.config).addr() - base) / size_of::<LayerConfig>();
            let p = layer.params;
            let value = VrLayerOut {
                layer_index: index as u32,
                kneeboard_pose: p.kneeboard_pose,
                kneeboard_size: p.kneeboard_size,
                kneeboard_opacity: p.kneeboard_opacity,
                is_looking_at_kneeboard: p.is_looking_at_kneeboard,
            };
            // SAFETY: `i < vr_layers.len() <= count <= 16`, and `out` is valid for 16 writes.
            unsafe { out.add(i).write(value) };
        }
        // SAFETY: valid for writes per the contract; checked non-null above.
        unsafe {
            out_count.write(vr_layers.len() as u32);
            active_view.write(match update {
                ActiveViewUpdate::NoCall => u64::MAX,
                ActiveViewUpdate::Set(id) => id,
            });
        }
    })
}

/// C++ `RayIntersectsRect`.
#[unsafe(no_mangle)]
pub extern "C" fn okb_ray_intersects_rect(
    ray_origin: Vec3,
    ray_orientation: Quat,
    rect_center: Vec3,
    rect_orientation: Quat,
    rect_size: Vec2,
) -> bool {
    ffi(|| {
        okb_vr::ray_intersects_rect(
            ray_origin,
            ray_orientation,
            rect_center,
            rect_orientation,
            rect_size,
        )
    })
}

/// C++ `SpriteAtlas`. Writes the atlas size to `*out_size` and one rect per sprite to
/// `out_rects`; returns 0, or -1 if C++ would throw (the sprites do not fit).
///
/// # Safety
/// `sprites` points to `count` `OkbPixelSize`s (`count` is 1 to 16); `out_size` is valid for a
/// write; `out_rects` is valid for `count` writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn okb_sprite_atlas_new(
    sprites: *const PixelSize,
    count: usize,
    out_size: *mut PixelSize,
    out_rects: *mut PixelRect,
) -> i32 {
    ffi(|| {
        assert!(
            (1..=MAX_VIEW_COUNT).contains(&count) && !sprites.is_null(),
            "bad sprite count {count}"
        );
        assert!(!out_size.is_null() && !out_rects.is_null(), "null out");
        // SAFETY: `sprites` points to `count` initialized values per the contract.
        let sprites = unsafe { std::slice::from_raw_parts(sprites, count) };
        match SpriteAtlas::new(sprites) {
            Ok(atlas) => {
                // SAFETY: valid for writes per the contract; `i < count`.
                unsafe {
                    out_size.write(atlas.size());
                    for i in 0..count {
                        out_rects.add(i).write(atlas.rect(i));
                    }
                }
                0
            }
            Err(okb_vr::AtlasError::DoesNotFit) => -1,
        }
    })
}

/// C++ `PixelSize::ScaledToFit(container)` (ShrinkOrGrow).
#[unsafe(no_mangle)]
pub extern "C" fn okb_scaled_to_fit(size: PixelSize, container: PixelSize) -> PixelSize {
    ffi(|| okb_vr::scaled_to_fit(size, container))
}
