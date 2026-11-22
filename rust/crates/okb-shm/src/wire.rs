//! `#[repr(C)]` v1 wire types. Layout is checked against
//! `layout/shm-v1.json` (generated from the C++ side) by `tests/layout.rs`.

use core::f32::consts::PI;

use crate::{MAX_VIEW_COUNT, SWAPCHAIN_LENGTH};

/// `"OKBMagic"` as the little-endian `uint64_t` that C++ stores in `SharedData::mMagic`.
pub const MAGIC: u64 = u64::from_le_bytes(*b"OKBMagic");

/// Total SHM size; the C++ side static-asserts the same value on x86 and x64.
pub const SHM_SIZE: usize = 3024;

const _: () = assert!(size_of::<SharedData>() == SHM_SIZE);

/// C++ `SHM::Wire::HeaderFlags` (`ULONG`).
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeaderFlags(pub u32);

impl HeaderFlags {
    pub const FEEDER_ATTACHED: u32 = 1;
}

/// C++ `SHM::Wire::FixedSizeHandle`: a `HANDLE` that is always 64 bits.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FixedSizeHandle(pub u64);

/// C++ `Geometry2D::Size<float>`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SizeF32 {
    pub width: f32,
    pub height: f32,
}

/// C++ `Geometry2D::Point<uint32_t>` (`PixelPoint`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PixelPoint {
    pub x: u32,
    pub y: u32,
}

/// C++ `Geometry2D::Size<uint32_t>` (`PixelSize`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PixelSize {
    pub width: u32,
    pub height: u32,
}

/// C++ `Geometry2D::Rect<uint32_t>::Origin`.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Origin(pub i32);

impl Origin {
    pub const TOP_LEFT: Self = Self(0);
    pub const BOTTOM_LEFT: Self = Self(1);
}

impl Default for Origin {
    fn default() -> Self {
        Self::TOP_LEFT
    }
}

/// C++ `Geometry2D::Rect<uint32_t>` (`PixelRect`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PixelRect {
    pub offset: PixelPoint,
    pub size: PixelSize,
    pub origin: Origin,
}

/// C++ `VRPose`; meters and radians.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VrPose {
    pub x: f32,
    pub eye_y: f32,
    pub z: f32,
    pub rx: f32,
    pub ry: f32,
    pub rz: f32,
}

impl Default for VrPose {
    fn default() -> Self {
        Self {
            x: 0.15,
            eye_y: -0.7,
            z: -0.4,
            rx: -2.0 * PI / 5.0,
            ry: -PI / 32.0,
            rz: 0.0,
        }
    }
}

/// C++ `GazeTargetScale`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GazeTargetScale {
    pub vertical: f32,
    pub horizontal: f32,
}

impl Default for GazeTargetScale {
    fn default() -> Self {
        Self {
            vertical: 1.0,
            horizontal: 1.0,
        }
    }
}

/// C++ `VROpacitySettings`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VrOpacitySettings {
    pub normal: f32,
    pub gaze: f32,
}

impl Default for VrOpacitySettings {
    fn default() -> Self {
        Self {
            normal: 1.0,
            gaze: 1.0,
        }
    }
}

/// C++ `VRRenderSettings::Quirks::Upscaling`.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Upscaling(pub i32);

impl Upscaling {
    pub const AUTOMATIC: Self = Self(0);
    pub const ALWAYS_OFF: Self = Self(1);
    pub const ALWAYS_ON: Self = Self(2);
}

impl Default for Upscaling {
    fn default() -> Self {
        Self::AUTOMATIC
    }
}

/// C++ `VRRenderSettings::Quirks`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Quirks {
    pub openxr_upscaling: Upscaling,
}

/// C++ `VRRenderSettings`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VrRenderSettings {
    pub quirks: Quirks,
    pub enable_gaze_input_focus: bool,
    pub force_zoom: bool,
    pub recenter_count: u64,
}

impl Default for VrRenderSettings {
    fn default() -> Self {
        Self {
            quirks: Quirks::default(),
            enable_gaze_input_focus: true,
            force_zoom: false,
            recenter_count: 0,
        }
    }
}

/// C++ `SHM::VRLayer`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VrLayer {
    pub pose: VrPose,
    pub physical_size: SizeF32,
    pub enable_gaze_zoom: bool,
    pub zoom_scale: f32,
    pub gaze_target_scale: GazeTargetScale,
    pub opacity: VrOpacitySettings,
    pub location_on_texture: PixelRect,
}

impl Default for VrLayer {
    fn default() -> Self {
        Self {
            pose: VrPose::default(),
            physical_size: SizeF32::default(),
            enable_gaze_zoom: true,
            zoom_scale: 2.0,
            gaze_target_scale: GazeTargetScale::default(),
            opacity: VrOpacitySettings::default(),
            location_on_texture: PixelRect::default(),
        }
    }
}

/// C++ `SHM::Config`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    pub global_input_layer_id: u64,
    pub vr: VrRenderSettings,
    pub texture_size: PixelSize,
    pub tint: [f32; 4],
}

impl Default for Config {
    fn default() -> Self {
        Self {
            global_input_layer_id: 0,
            vr: VrRenderSettings::default(),
            texture_size: PixelSize::default(),
            tint: [1.0; 4],
        }
    }
}

/// C++ `SHM::LayerConfig`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayerConfig {
    pub layer_id: u64,
    pub vr_enabled: bool,
    pub vr: VrLayer,
}

/// C++ `SHM::Wire::SharedData::Frame`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    pub texture: FixedSizeHandle,
    pub fence: FixedSizeHandle,
    pub ready_read_fence_value: i64,
    pub config: Config,
    pub layers: [LayerConfig; MAX_VIEW_COUNT],
    pub layer_count: u32,
}

/// C++ `SHM::Wire::SharedData`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SharedData {
    pub magic: u64,
    pub gpu_luid: u64,
    pub frame_number: u64,
    pub session_id: u64,
    pub flags: HeaderFlags,
    pub feeder_process_id: u32,
    pub padding: [u8; 8],
    pub frames: [Frame; SWAPCHAIN_LENGTH],
}

impl SharedData {
    /// Magic set, everything else default.
    pub fn new(session_id: u64) -> Self {
        Self {
            magic: MAGIC,
            session_id,
            ..Self::default()
        }
    }

    /// C++ `SharedData::HaveFeeder()`.
    pub fn have_feeder(&self) -> bool {
        self.magic == MAGIC && self.flags.0 & HeaderFlags::FEEDER_ATTACHED != 0
    }
}

/// C++ `SHM::ActiveConsumers`; time points are `steady_clock` nanoseconds.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActiveConsumers {
    pub open_vr: i64,
    pub open_xr_d3d11: i64,
    pub open_xr_d3d12: i64,
    pub open_xr_vulkan2: i64,
    pub viewer: i64,
    pub elevated_consumer_process_id: u32,
    pub non_vr_pixel_size: PixelSize,
    pub global_input_view_id: u64,
    pub active_view_id: u64,
}
