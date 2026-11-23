//! Port of the C++ `VRKneeboard` and `RayIntersectsRect` (`src/lib`): kneeboard placement and
//! gaze detection, with the DirectXMath/SimpleMath operations they use hand-ported in `f32`.
#![forbid(unsafe_code)]

mod math;
mod ray;
mod vr_kneeboard;

pub use math::{Pose, Quat, Vec2, Vec3};
pub use ray::ray_intersects_rect;
pub use vr_kneeboard::{ActiveViewUpdate, RenderParameters, VrKneeboard, VrLayer};
