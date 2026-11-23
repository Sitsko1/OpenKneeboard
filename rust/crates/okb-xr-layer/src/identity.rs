//! Layer identity and SHM names, from the CMake (Corrosion) environment, with
//! defaults matching the top-level `CMakeLists.txt` for standalone cargo.
#![forbid(unsafe_code)]

const fn env_or(value: Option<&'static str>, default: &'static str) -> &'static str {
    match value {
        Some(v) => v,
        None => default,
    }
}

const fn env_u32(value: Option<&str>, default: u32) -> u32 {
    match value {
        Some(v) => match u32::from_str_radix(v, 10) {
            Ok(n) => n,
            Err(_) => panic!("identity env var is not a u32"),
        },
        None => default,
    }
}

pub const LAYER_NAME: &str = env_or(
    option_env!("OKB_OPENXR_LAYER_NAME"),
    "XR_APILAYER_FREDEMMOTT_OpenKneeboard",
);
pub const LAYER_DESCRIPTION: &str = env_or(
    option_env!("OKB_OPENXR_LAYER_DESCRIPTION"),
    "OpenKneeboard - https://github.com/OpenKneeboard/OpenKneeboard",
);

const REVERSE_DOMAIN: &str = env_or(option_env!("OKB_REVERSE_DOMAIN"), "com.openkneeboard");
const VERSION_MAJOR: u32 = env_u32(option_env!("OKB_VERSION_MAJOR"), 1);
const VERSION_MINOR: u32 = env_u32(option_env!("OKB_VERSION_MINOR"), 13);
const VERSION_PATCH: u32 = env_u32(option_env!("OKB_VERSION_PATCH"), 0);
const VERSION_BUILD: u32 = env_u32(option_env!("OKB_VERSION_BUILD"), 0);

/// CMake: `(major << 24) | minor`.
pub const LAYER_IMPL_VERSION: u32 = env_u32(
    option_env!("OKB_OPENXR_LAYER_IMPL_VERSION"),
    (VERSION_MAJOR << 24) | VERSION_MINOR,
);

pub fn shm_names() -> okb_shm::Names {
    okb_shm::Names::new(
        REVERSE_DOMAIN,
        okb_shm::Version {
            major: VERSION_MAJOR,
            minor: VERSION_MINOR,
            patch: VERSION_PATCH,
            build: VERSION_BUILD,
        },
    )
}
