//! SHM frame writer, mirroring C++ `SHM::Writer` (`src/lib/SHM.cpp`).

use crate::win32::{LockOutcome, Mapping, NamedMutex, session_id};
use crate::wire::{Config, FixedSizeHandle, Frame, HeaderFlags, LayerConfig, SHM_SIZE, SharedData};
use crate::{MAX_VIEW_COUNT, Names, SWAPCHAIN_LENGTH};

/// C++ `Writer::NextFrameInfo`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NextFrameInfo {
    pub texture_index: u8,
    pub fence_out: i64,
}

/// The feeder side of the SHM. `!Send`: the mutex must be released on the locking thread.
pub struct Writer {
    mapping: Mapping,
    mutex: NamedMutex,
    process_id: u32,
    gpu_luid: u64,
    ready_read_fence_value: i64,
}

impl Writer {
    /// `None` where C++ `Writer::operator bool` is false.
    ///
    /// Overwrites the header without taking the lock, as the C++ constructor does.
    pub fn new(names: &Names, gpu_luid: u64) -> Option<Self> {
        let (mapping, _created) = Mapping::open(&names.frames, SHM_SIZE).ok()?;
        let mutex = NamedMutex::open(&names.mutex).ok()?;
        mapping.write_header(&SharedData::new(session_id()));
        Some(Self {
            mapping,
            mutex,
            process_id: std::process::id(),
            gpu_luid,
            ready_read_fence_value: 0,
        })
    }

    /// Blocks until the mutex is held.
    ///
    /// # Panics
    /// If the wait fails (C++ breaks into the debugger and returns unlocked).
    pub fn lock(&mut self) -> WriterGuard<'_> {
        let outcome = self.mutex.lock().expect("SHM mutex wait failed");
        self.guard(outcome)
    }

    /// `None` if another process holds the mutex (or the wait failed).
    pub fn try_lock(&mut self) -> Option<WriterGuard<'_>> {
        let outcome = self.mutex.try_lock().ok()??;
        Some(self.guard(outcome))
    }

    /// C++ `Writer::GetFrameCountForMetricsOnly`; unlocked.
    pub fn frame_count(&self) -> u64 {
        self.mapping.read_header().frame_number
    }

    fn guard(&mut self, outcome: LockOutcome) -> WriterGuard<'_> {
        if outcome == LockOutcome::Abandoned {
            // C++ `*mHeader = {}`, which also creates a fresh session ID.
            self.mapping.write_header(&SharedData::new(session_id()));
        }
        WriterGuard {
            writer: self,
            frame_in_progress: false,
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.lock().detach();
    }
}

/// The C++ `Locked` state: holds the SHM mutex until dropped.
pub struct WriterGuard<'a> {
    writer: &'a mut Writer,
    frame_in_progress: bool,
}

impl WriterGuard<'_> {
    pub fn submit_empty_frame(&mut self) {
        let mut header = self.writer.mapping.read_header();
        header.frame_number += 1;
        header.frames[header.frame_number as usize % SWAPCHAIN_LENGTH].layer_count = 0;
        self.writer.mapping.write_header(&header);
    }

    pub fn begin_frame(&mut self) -> NextFrameInfo {
        self.frame_in_progress = true;
        let frame_number = self.frame_count();
        self.writer.ready_read_fence_value += 1;
        NextFrameInfo {
            texture_index: ((frame_number + 1) % SWAPCHAIN_LENGTH as u64) as u8,
            fence_out: self.writer.ready_read_fence_value,
        }
    }

    /// # Panics
    /// If `layers.len() > MAX_VIEW_COUNT` (C++ `fatal()`).
    pub fn submit_frame(
        &mut self,
        info: NextFrameInfo,
        config: &Config,
        layers: &[LayerConfig],
        texture: u64,
        fence: u64,
    ) {
        debug_assert!(self.frame_in_progress, "submit_frame without begin_frame");
        self.frame_in_progress = false;
        assert!(
            layers.len() <= MAX_VIEW_COUNT,
            "Asked to publish {} layers, but max is {MAX_VIEW_COUNT}",
            layers.len()
        );

        let w = &*self.writer;
        let mut header = w.mapping.read_header();
        header.gpu_luid = w.gpu_luid;
        header.flags.0 |= HeaderFlags::FEEDER_ATTACHED;
        header.feeder_process_id = w.process_id;
        header.frame_number += 1;
        let idx = header.frame_number as usize % SWAPCHAIN_LENGTH;
        assert_eq!(idx, usize::from(info.texture_index));
        assert_eq!(w.ready_read_fence_value, info.fence_out);

        let mut frame = Frame {
            texture: FixedSizeHandle(texture),
            fence: FixedSizeHandle(fence),
            ready_read_fence_value: info.fence_out,
            config: *config,
            layer_count: layers.len() as u32,
            ..Frame::default()
        };
        frame.layers[..layers.len()].copy_from_slice(layers);
        header.frames[idx] = frame;
        w.mapping.write_header(&header);
    }

    /// Resets the header with a new session ID, so consumers see the feeder go away.
    pub fn detach(&mut self) {
        self.writer
            .mapping
            .write_header(&SharedData::new(session_id()));
        self.writer.mapping.flush();
    }

    pub fn frame_count(&self) -> u64 {
        self.writer.mapping.read_header().frame_number
    }
}

impl Drop for WriterGuard<'_> {
    fn drop(&mut self) {
        self.writer.mutex.unlock();
    }
}

#[cfg(test)]
mod tests {
    use std::mem::offset_of;

    use super::*;
    use crate::Version;

    fn writer(test: &str, luid: u64) -> (Writer, Names) {
        let names = Names::new(
            &format!("okb-test.{test}.{}", std::process::id()),
            Version {
                major: 1,
                minor: 0,
                patch: 0,
                build: 0,
            },
        );
        (Writer::new(&names, luid).expect("writer"), names)
    }

    fn layer(id: u64) -> LayerConfig {
        LayerConfig {
            layer_id: id,
            vr_enabled: true.into(),
            ..LayerConfig::default()
        }
    }

    #[test]
    fn empty_frame_does_not_attach_feeder() {
        let (mut w, _) = writer("empty", 1);
        let mut g = w.lock();
        g.submit_empty_frame();
        let h = g.writer.mapping.read_header();
        assert!(!h.have_feeder());
        assert_eq!(h.frame_number, 1);
    }

    #[test]
    fn submit_frame_sets_feeder_fields() {
        let (mut w, _) = writer("submit", 0xABCD);
        let mut g = w.lock();
        let info = g.begin_frame();
        let config = Config {
            global_input_layer_id: 7,
            ..Config::default()
        };
        let layers = [layer(1), layer(2)];
        g.submit_frame(info, &config, &layers, 0x11, 0x22);
        let h = g.writer.mapping.read_header();
        assert_eq!(h.gpu_luid, 0xABCD);
        assert!(h.have_feeder());
        assert_eq!(h.flags.0 & HeaderFlags::FEEDER_ATTACHED, 1);
        assert_eq!(h.feeder_process_id, std::process::id());
        assert_eq!(h.frame_number, 1);
        let f = &h.frames[1];
        assert_eq!(f.texture, FixedSizeHandle(0x11));
        assert_eq!(f.fence, FixedSizeHandle(0x22));
        assert_eq!(f.config, config);
        assert_eq!(f.layer_count, 2);
        assert_eq!(f.layers[..2], layers);
        assert_eq!(f.layers[2], LayerConfig::default());
        assert_eq!(f.ready_read_fence_value, info.fence_out);
    }

    #[test]
    fn begin_frame_index_formula() {
        let (mut w, _) = writer("begin", 1);
        let mut g = w.lock();
        for n in 0..4u64 {
            let info = g.begin_frame();
            assert_eq!(u64::from(info.texture_index), (n + 1) % 2);
            assert_eq!(info.fence_out, n as i64 + 1);
            g.submit_frame(info, &Config::default(), &[], 0, 0);
        }
    }

    #[test]
    fn detach_creates_new_session_and_clears_feeder() {
        let (mut w, _) = writer("detach", 1);
        let mut g = w.lock();
        let info = g.begin_frame();
        g.submit_frame(info, &Config::default(), &[layer(1)], 1, 2);
        let old = g.writer.mapping.read_header();
        assert!(old.have_feeder());
        g.detach();
        let h = g.writer.mapping.read_header();
        assert!(!h.have_feeder());
        assert_eq!(h.magic, crate::wire::MAGIC);
        assert_eq!(h.frame_number, 0);
        assert_ne!(h.session_id, old.session_id);
    }

    #[test]
    fn abandoned_lock_resets_header() {
        let (mut w, names) = writer("abandoned", 1);
        {
            let mut g = w.lock();
            let info = g.begin_frame();
            g.submit_frame(info, &Config::default(), &[], 1, 2);
        }
        let old = w.mapping.read_header();
        assert!(old.have_feeder());
        let mutex_name = names.mutex.clone();
        std::thread::spawn(move || {
            let m = NamedMutex::open(&mutex_name).unwrap();
            assert_eq!(m.lock().unwrap(), LockOutcome::Acquired);
            // exit without unlocking
        })
        .join()
        .unwrap();
        let g = w.lock();
        let h = g.writer.mapping.read_header();
        assert!(!h.have_feeder());
        assert_eq!(h.magic, crate::wire::MAGIC);
        assert_eq!(h.frame_number, 0);
        assert_ne!(h.session_id, old.session_id);
    }

    /// Byte ranges of the implicit C++ padding in `frames[frame]`, found from the neighboring
    /// fields' offsets.
    fn frame_padding(frame: usize) -> Vec<std::ops::Range<usize>> {
        use crate::wire::{VrLayer, VrRenderSettings};
        let base = offset_of!(SharedData, frames) + frame * size_of::<Frame>();
        let vr = base + offset_of!(Frame, config) + offset_of!(Config, vr);
        let mut ranges = vec![
            vr + offset_of!(VrRenderSettings, force_zoom) + 1
                ..vr + offset_of!(VrRenderSettings, recenter_count),
            base + offset_of!(Frame, layer_count) + 4..base + size_of::<Frame>(),
        ];
        for i in 0..MAX_VIEW_COUNT {
            let layer = base + offset_of!(Frame, layers) + i * size_of::<LayerConfig>();
            let vr = layer + offset_of!(LayerConfig, vr);
            ranges.push(layer + offset_of!(LayerConfig, vr_enabled) + 1..vr);
            ranges.push(
                vr + offset_of!(VrLayer, enable_gaze_zoom) + 1
                    ..vr + offset_of!(VrLayer, zoom_scale),
            );
        }
        ranges
    }

    fn assert_zero(bytes: &[u8], ranges: &[std::ops::Range<usize>]) {
        for r in ranges {
            assert!(
                bytes[r.clone()].iter().all(|&b| b == 0),
                "padding {r:?}: {:x?}",
                &bytes[r.clone()]
            );
        }
    }

    // C++ value-initializes the header and frame, so padding is zero; the Rust writer used to
    // copy stack garbage there (found by SHMParity.* in src/tests/SHMParity_test.cpp).
    #[test]
    fn padding_bytes_are_written_as_zero() {
        let (mut w, _) = writer("padding", 1);
        w.mapping.fill_from(offset_of!(SharedData, frames), 0xFF);
        let mut g = w.lock();
        let info = g.begin_frame();
        g.submit_frame(info, &Config::default(), &[layer(1), layer(2)], 1, 2);
        assert_zero(
            &g.writer.mapping.bytes(),
            &frame_padding(usize::from(info.texture_index)),
        );

        g.writer
            .mapping
            .fill_from(offset_of!(SharedData, frames), 0xFF);
        g.detach();
        let bytes = g.writer.mapping.bytes();
        for frame in 0..SWAPCHAIN_LENGTH {
            assert_zero(&bytes, &frame_padding(frame));
        }
    }

    #[test]
    #[should_panic(expected = "max is 16")]
    fn submit_frame_panics_over_max_view_count() {
        let (mut w, _) = writer("overmax", 1);
        let mut g = w.lock();
        let info = g.begin_frame();
        let layers = vec![LayerConfig::default(); MAX_VIEW_COUNT + 1];
        g.submit_frame(info, &Config::default(), &layers, 0, 0);
    }

    #[test]
    #[should_panic(expected = "assertion `left == right` failed")]
    fn submit_frame_panics_on_mismatched_texture_index() {
        let (mut w, _) = writer("badindex", 1);
        let mut g = w.lock();
        let mut info = g.begin_frame();
        info.texture_index ^= 1;
        g.submit_frame(info, &Config::default(), &[], 0, 0);
    }
}
