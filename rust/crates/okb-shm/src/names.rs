//! SHM object names, byte-identical to the C++ `SHMPath()`/`MutexPath()`/
//! `ActiveConsumers::GetSHMPath()`.

use crate::wire::{self, SHM_SIZE};

/// Application version, as embedded in the object names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub build: u32,
}

/// Names of the named kernel objects used by the SHM transport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Names {
    pub frames: String,
    pub mutex: String,
    pub active_consumers: String,
}

impl Names {
    pub fn new(reverse_domain: &str, v: Version) -> Self {
        let Version {
            major,
            minor,
            patch,
            build,
        } = v;
        let prefix = format!("{reverse_domain}/{major}.{minor}.{patch}.{build}");
        let frames = format!("{prefix}-s{SHM_SIZE:x}");
        Self {
            mutex: format!("{frames}.mutex"),
            frames,
            active_consumers: format!(
                "{reverse_domain}/{major}.{minor}.{patch}.{build}/ActiveConsumers-s{:x}",
                size_of::<wire::ActiveConsumers>()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_cpp_format() {
        let n = Names::new(
            "com.openkneeboard",
            Version {
                major: 1,
                minor: 13,
                patch: 0,
                build: 0,
            },
        );
        assert_eq!(n.frames, "com.openkneeboard/1.13.0.0-sbd0");
        assert_eq!(n.mutex, "com.openkneeboard/1.13.0.0-sbd0.mutex");
        assert_eq!(
            n.active_consumers,
            format!(
                "com.openkneeboard/1.13.0.0/ActiveConsumers-s{:x}",
                size_of::<wire::ActiveConsumers>()
            )
        );
    }
}
