//! SHM frame reader, mirroring C++ `SHM::Reader` (`src/lib/SHM.cpp`).

use std::io;

use crate::active_consumers::{self, ConsumerKind};
use crate::win32::{
    LockOutcome, Mapping, NamedMutex, OwnedHandle, duplicate_handle, open_process_for_dup, raw,
    session_id,
};
use crate::wire::{Config, LayerConfig, SHM_SIZE, SharedData};
use crate::{MAX_VIEW_COUNT, Names, SWAPCHAIN_LENGTH};

/// C++ `Frame::Error`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    IncorrectGpu,
    NoFeeder,
    NoLayers,
    UnusableHandles,
}

/// Why [`Reader::maybe_get`] returned no frame.
#[derive(Debug)]
pub enum ReadError {
    Frame(FrameError),
    /// `DuplicateHandle` failed; C++ throws here.
    DuplicateHandle(io::Error),
}

impl From<FrameError> for ReadError {
    fn from(e: FrameError) -> Self {
        Self::Frame(e)
    }
}

/// C++ `SHM::Frame`. `texture` and `fence` are raw values of handles owned by the
/// [`Reader`]; they stay valid until the next session change or until the reader is dropped.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub config: Config,
    pub layers: Vec<LayerConfig>,
    pub texture: u64,
    pub fence: u64,
    pub fence_in: i64,
    pub index: u8,
}

/// C++ `Reader::Impl::IPCHandle`.
#[derive(Default)]
struct IpcHandle {
    foreign: u64,
    mine: Option<OwnedHandle>,
}

impl IpcHandle {
    fn update(&mut self, from: &OwnedHandle, foreign: u64) -> io::Result<()> {
        if foreign == self.foreign && self.mine.is_some() {
            return Ok(());
        }
        self.foreign = foreign;
        // `std::out_ptr` releases the old handle before `DuplicateHandle` writes the new one.
        self.mine = None;
        self.mine = Some(duplicate_handle(from, foreign)?);
        Ok(())
    }
}

/// C++ `Reader::Impl::SessionResources`.
#[derive(Default)]
struct SessionResources {
    feeder: Option<OwnedHandle>,
    /// `[texture, fence]` per swapchain slot.
    frames: [[IpcHandle; 2]; SWAPCHAIN_LENGTH],
}

/// The consumer side of the SHM. `!Send`: the mutex must be released on the locking thread.
pub struct Reader<F: FnMut()> {
    mapping: Mapping,
    mutex: NamedMutex,
    names: Names,
    kind: ConsumerKind,
    gpu_luid: u64,
    on_session_changed: F,
    session_id: u64,
    session: SessionResources,
}

/// Holds the SHM mutex; releases it on drop, so no early return leaves it held.
struct Locked<'a>(&'a NamedMutex);

impl<'a> Locked<'a> {
    /// C++ `Impl::lock()`, including the abandoned-mutex header reset.
    ///
    /// # Panics
    /// If the wait fails, as `Writer::lock` does.
    fn new(mutex: &'a NamedMutex, mapping: &Mapping) -> Self {
        if mutex.lock().expect("SHM mutex wait failed") == LockOutcome::Abandoned {
            mapping.write_header(&SharedData::new(session_id()));
        }
        Self(mutex)
    }
}

impl Drop for Locked<'_> {
    fn drop(&mut self) {
        self.0.unlock();
    }
}

impl<F: FnMut()> Reader<F> {
    /// `None` where the C++ reader's `p` is null (mapping or mutex unavailable).
    pub fn new(
        names: &Names,
        kind: ConsumerKind,
        gpu_luid: u64,
        on_session_changed: F,
    ) -> Option<Self> {
        let (mapping, _created) = Mapping::open(&names.frames, SHM_SIZE).ok()?;
        let mutex = NamedMutex::open(&names.mutex).ok()?;
        Some(Self {
            mapping,
            mutex,
            names: names.clone(),
            kind,
            gpu_luid,
            on_session_changed,
            session_id: u64::MAX,
            session: SessionResources::default(),
        })
    }

    /// C++ `Reader::operator bool`; unlocked.
    pub fn has_feeder(&self) -> bool {
        self.mapping.read_header().have_feeder()
    }

    /// C++ `Reader::MaybeGet`, with the same check order.
    pub fn maybe_get(&mut self) -> Result<Frame, ReadError> {
        // Quick check without the lock; repeated once locked.
        if !self.has_feeder() {
            return Err(FrameError::NoFeeder.into());
        }
        let _lock = Locked::new(&self.mutex, &self.mapping);
        let header = self.mapping.read_header();
        if !header.have_feeder() {
            return Err(FrameError::NoFeeder.into());
        }

        let previous = self.session_id;
        // As in C++, a failed duplication (a throw there) skips `on_session_changed`.
        update_session(&header, &mut self.session_id, &mut self.session)
            .map_err(ReadError::DuplicateHandle)?;
        if self.session_id != previous {
            (self.on_session_changed)();
        }

        if header.gpu_luid != self.gpu_luid {
            return Err(FrameError::IncorrectGpu.into());
        }

        active_consumers::set(&self.names, self.kind);

        let index = (header.frame_number % SWAPCHAIN_LENGTH as u64) as usize;
        let frame = &header.frames[index];
        if frame.layer_count == 0 {
            return Err(FrameError::NoLayers.into());
        }

        let [Some(texture), Some(fence)] = self.session.frames[index]
            .each_ref()
            .map(|h| h.mine.as_ref())
        else {
            return Err(FrameError::UnusableHandles.into());
        };

        Ok(Frame {
            config: frame.config,
            // Clamped: a peer-written count above the array would be UB in C++, a panic here.
            layers: frame.layers[..(frame.layer_count as usize).min(MAX_VIEW_COUNT)].to_vec(),
            texture: raw(texture),
            fence: raw(fence),
            fence_in: frame.ready_read_fence_value,
            index: index as u8,
        })
    }

    /// C++ `Reader::GetFrameCountForMetricsOnly`; unlocked.
    pub fn frame_count(&self) -> u64 {
        self.mapping.read_header().frame_number
    }
}

/// C++ `Reader::Impl::UpdateSession`.
fn update_session(
    shared: &SharedData,
    session_id: &mut u64,
    session: &mut SessionResources,
) -> io::Result<()> {
    if shared.session_id != *session_id {
        *session = SessionResources::default();
        *session_id = shared.session_id;
    }
    if *session_id == 0 {
        return Ok(());
    }
    if session.feeder.is_none() {
        session.feeder = open_process_for_dup(shared.feeder_process_id);
    }
    let Some(feeder) = &session.feeder else {
        return Ok(());
    };
    let index = (shared.frame_number % SWAPCHAIN_LENGTH as u64) as usize;
    let source = &shared.frames[index];
    if source.layer_count == 0 {
        // Nothing to render, and the handles may not be valid.
        return Ok(());
    }
    let [texture, fence] = &mut session.frames[index];
    texture.update(feeder, source.texture.0)?;
    fence.update(feeder, source.fence.0)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::wire::{FixedSizeHandle, HeaderFlags};
    use crate::{Version, Writer, win32};

    fn names(test: &str) -> Names {
        Names::new(
            &format!("okb-test.{test}.{}", std::process::id()),
            Version {
                major: 1,
                minor: 0,
                patch: 0,
                build: 0,
            },
        )
    }

    fn reader(names: &Names, luid: u64) -> Reader<impl FnMut()> {
        Reader::new(names, ConsumerKind::OpenXrD3d11, luid, || {}).expect("reader")
    }

    fn frame_err(r: Result<Frame, ReadError>) -> FrameError {
        match r {
            Err(ReadError::Frame(e)) => e,
            Err(e) => panic!("unexpected {e:?}"),
            Ok(f) => panic!("unexpected frame {f:?}"),
        }
    }

    fn layer(id: u64) -> LayerConfig {
        LayerConfig {
            layer_id: id,
            vr_enabled: true.into(),
            ..LayerConfig::default()
        }
    }

    #[test]
    fn error_precedence_matches_cpp() {
        let names = names("precedence");
        let mut wrong = reader(&names, 1);
        let mut right = reader(&names, 2);
        assert_eq!(frame_err(wrong.maybe_get()), FrameError::NoFeeder);
        assert!(!wrong.has_feeder());

        let mut w = Writer::new(&names, 2).unwrap();
        w.lock().submit_empty_frame();
        assert_eq!(frame_err(wrong.maybe_get()), FrameError::NoFeeder);

        {
            let mut g = w.lock();
            let info = g.begin_frame();
            g.submit_frame(info, &Config::default(), &[], 0, 0);
        }
        assert!(wrong.has_feeder());
        assert_eq!(frame_err(wrong.maybe_get()), FrameError::IncorrectGpu);
        assert_eq!(frame_err(right.maybe_get()), FrameError::NoLayers);
    }

    #[test]
    fn usable_handles_round_trip() {
        let names = names("roundtrip");
        let texture = win32::create_event();
        let fence = win32::create_event();
        let mut w = Writer::new(&names, 7).unwrap();
        let config = Config {
            global_input_layer_id: 3,
            ..Config::default()
        };
        let layers = [layer(10), layer(11)];
        let info = {
            let mut g = w.lock();
            let info = g.begin_frame();
            g.submit_frame(
                info,
                &config,
                &layers,
                win32::raw(&texture),
                win32::raw(&fence),
            );
            info
        };

        let mut r = reader(&names, 7);
        let f = r.maybe_get().expect("frame");
        assert_ne!(f.texture, 0);
        assert_ne!(f.fence, 0);
        assert!(win32::same_object(f.texture, win32::raw(&texture)));
        assert!(win32::same_object(f.fence, win32::raw(&fence)));
        assert_eq!(f.config, config);
        assert_eq!(f.layers, layers);
        assert_eq!(f.fence_in, info.fence_out);
        assert_eq!(u64::from(f.index), r.frame_count() % 2);
        assert_eq!(f.index, info.texture_index);
        assert_ne!(active_consumers::get(&names).open_xr_d3d11, 0);

        // Same foreign handles: no re-duplication, so the local values are stable.
        let again = r.maybe_get().expect("frame");
        assert_eq!((again.texture, again.fence), (f.texture, f.fence));
    }

    #[test]
    fn session_callback_fires_once_per_session() {
        let names = names("session");
        let calls = Cell::new(0);
        let mut r = Reader::new(&names, ConsumerKind::Viewer, 5, || {
            calls.set(calls.get() + 1);
        })
        .unwrap();
        let mut w = Writer::new(&names, 5).unwrap();
        let submit = |w: &mut Writer| {
            let mut g = w.lock();
            let info = g.begin_frame();
            g.submit_frame(info, &Config::default(), &[], 0, 0);
        };

        submit(&mut w);
        assert_eq!(frame_err(r.maybe_get()), FrameError::NoLayers);
        assert_eq!(frame_err(r.maybe_get()), FrameError::NoLayers);
        assert_eq!(calls.get(), 1);

        w.lock().detach();
        assert_eq!(frame_err(r.maybe_get()), FrameError::NoFeeder);
        assert_eq!(calls.get(), 1);
        submit(&mut w);
        assert_eq!(frame_err(r.maybe_get()), FrameError::NoLayers);
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn dead_feeder_pid_gives_unusable_handles() {
        let names = names("deadpid");
        let mut r = reader(&names, 9);
        let (mapping, _) = win32::Mapping::open(&names.frames, crate::wire::SHM_SIZE).unwrap();
        let mut h = SharedData::new(1);
        h.flags = HeaderFlags(HeaderFlags::FEEDER_ATTACHED);
        h.gpu_luid = 9;
        h.frame_number = 1;
        h.feeder_process_id = 0;
        h.frames[1].layer_count = 1;
        h.frames[1].texture = FixedSizeHandle(0x1234);
        h.frames[1].fence = FixedSizeHandle(0x5678);
        mapping.write_header(&h);
        assert_eq!(frame_err(r.maybe_get()), FrameError::UnusableHandles);
    }
}
