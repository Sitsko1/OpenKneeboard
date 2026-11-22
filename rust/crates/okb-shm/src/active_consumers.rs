//! C++ `SHM::ActiveConsumers` (`src/lib/SHM/ActiveConsumers.cpp`).

use std::collections::BTreeMap;
use std::mem::offset_of;
use std::sync::{Mutex, PoisonError};

use crate::Names;
use crate::win32::{Mapping, is_elevated, steady_clock_now_ns};
pub use crate::wire::ActiveConsumers;

/// C++ `SHM::ConsumerKind`.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsumerKind {
    OpenVr = 1,
    OpenXrD3d11 = 2,
    OpenXrD3d12 = 4,
    OpenXrVulkan2 = 8,
    Viewer = u32::MAX,
}

struct Entry {
    mapping: Mapping,
    have_checked_elevation: bool,
}

/// C++ `static Impl sImpl`: one mapping per process, here per name. A failed open is cached
/// too (`None`), as the C++ static is never retried.
static MAPPINGS: Mutex<BTreeMap<String, Option<Entry>>> = Mutex::new(BTreeMap::new());

fn with_entry(names: &Names, f: impl FnOnce(&mut Entry)) {
    let mut map = MAPPINGS.lock().unwrap_or_else(PoisonError::into_inner);
    let entry = map
        .entry(names.active_consumers.clone())
        .or_insert_with(|| {
            let (mapping, created) =
                Mapping::open(&names.active_consumers, size_of::<ActiveConsumers>()).ok()?;
            if created {
                mapping.zero_active_consumers();
            }
            Some(Entry {
                mapping,
                have_checked_elevation: false,
            })
        });
    if let Some(entry) = entry {
        f(entry);
    }
}

/// C++ `ActiveConsumers::Set`. `OpenXrD3d12` writes the D3D11 timestamp, as the C++ does.
pub fn set(names: &Names, kind: ConsumerKind) {
    with_entry(names, |e| {
        if !e.have_checked_elevation {
            if is_elevated() {
                e.mapping.write_at(
                    offset_of!(ActiveConsumers, elevated_consumer_process_id),
                    std::process::id(),
                );
            }
            e.have_checked_elevation = true;
        }
        let offset = match kind {
            ConsumerKind::OpenVr => offset_of!(ActiveConsumers, open_vr),
            // ponytail: C++ quirk preserved; D3D12 consumers report as D3D11.
            ConsumerKind::OpenXrD3d11 | ConsumerKind::OpenXrD3d12 => {
                offset_of!(ActiveConsumers, open_xr_d3d11)
            }
            ConsumerKind::OpenXrVulkan2 => offset_of!(ActiveConsumers, open_xr_vulkan2),
            ConsumerKind::Viewer => offset_of!(ActiveConsumers, viewer),
        };
        e.mapping.write_at(offset, steady_clock_now_ns());
    });
}

/// C++ `ActiveConsumers::SetActiveInGameViewID`: a non-zero id also becomes the global
/// input view.
pub fn set_active_in_game_view_id(names: &Names, id: u64) {
    with_entry(names, |e| {
        e.mapping
            .write_at(offset_of!(ActiveConsumers, active_view_id), id);
        if id != 0 {
            e.mapping
                .write_at(offset_of!(ActiveConsumers, global_input_view_id), id);
        }
    });
}

/// C++ `ActiveConsumers::Get`: a copy, or the default if the mapping is unavailable.
pub fn get(names: &Names) -> ActiveConsumers {
    let mut out = ActiveConsumers::default();
    with_entry(names, |e| out = e.mapping.read_active_consumers());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Version;

    #[test]
    fn d3d12_consumer_sets_d3d11_timestamp_quirk() {
        let names = Names::new(
            &format!("okb-test.quirk.{}", std::process::id()),
            Version {
                major: 1,
                minor: 0,
                patch: 0,
                build: 0,
            },
        );
        assert_eq!(get(&names), ActiveConsumers::default());
        set(&names, ConsumerKind::OpenXrD3d12);
        let c = get(&names);
        assert_ne!(c.open_xr_d3d11, 0);
        assert_eq!(c.open_xr_d3d12, 0);

        set_active_in_game_view_id(&names, 5);
        set_active_in_game_view_id(&names, 0);
        let c = get(&names);
        assert_eq!(c.active_view_id, 0);
        assert_eq!(c.global_input_view_id, 5);
    }
}
