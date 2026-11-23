use crate::math::{Plane, Quat, Vec2, Vec3, ray_intersects_plane};

/// C++ `RayIntersectsRect`: does the ray from `ray_origin` along the pose's forward (-Z) axis
/// hit the rectangle centered at `rect_center`, facing along the rect pose's +Z axis?
pub fn ray_intersects_rect(
    ray_origin: Vec3,
    ray_orientation: Quat,
    rect_center: Vec3,
    rect_orientation: Quat,
    rect_size: Vec2,
) -> bool {
    let ray_normal = Vec3::FORWARD.rotate(ray_orientation);
    let plane = Plane::from_point_normal(rect_center, Vec3::BACKWARD.rotate(rect_orientation));

    // Does the ray intersect the infinite plane?
    let Some(ray_length) = ray_intersects_plane(ray_origin, ray_normal, plane) else {
        return false;
    };

    // Where does it intersect the infinite plane?
    let world_point = ray_origin.add(ray_normal.scale(ray_length));

    // Is that point within the rectangle?
    let point = world_point.sub(rect_center);

    let x = point.dot(Vec3::UNIT_X.rotate(rect_orientation));
    if x.abs() > rect_size.x / 2.0 {
        return false;
    }

    let y = point.dot(Vec3::UNIT_Y.rotate(rect_orientation));
    if y.abs() > rect_size.y / 2.0 {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    //! Mirrors `src/tests/RayIntersectsRect_test.cpp`.
    use std::f32::consts::PI;

    use super::*;

    // Rect 1 m x 1 m at (0, 0, -2), facing the viewer (identity orientation).
    // OpenXR convention: identity orientation looks down -Z.
    const RECT_CENTER: Vec3 = Vec3::new(0.0, 0.0, -2.0);
    const RECT_SIZE: Vec2 = Vec2::new(1.0, 1.0);
    const ORIGIN: Vec3 = Vec3::new(0.0, 0.0, 0.0);

    fn yaw(radians: f32) -> Quat {
        Quat::from_yaw_pitch_roll(radians, 0.0, 0.0)
    }

    fn hits(origin: Vec3, ray: Quat, center: Vec3, rect: Quat) -> bool {
        ray_intersects_rect(origin, ray, center, rect, RECT_SIZE)
    }

    #[test]
    fn hits_center_straight_ahead() {
        assert!(hits(ORIGIN, Quat::IDENTITY, RECT_CENTER, Quat::IDENTITY));
    }

    #[test]
    fn misses_when_pointing_away() {
        assert!(!hits(ORIGIN, yaw(PI), RECT_CENTER, Quat::IDENTITY));
    }

    #[test]
    fn hits_inside_edge() {
        let origin = Vec3::new(0.49, 0.0, 0.0);
        assert!(hits(origin, Quat::IDENTITY, RECT_CENTER, Quat::IDENTITY));
    }

    #[test]
    fn misses_outside_edge() {
        let origin = Vec3::new(0.51, 0.0, 0.0);
        assert!(!hits(origin, Quat::IDENTITY, RECT_CENTER, Quat::IDENTITY));
    }

    #[test]
    fn misses_when_rect_behind_origin() {
        let behind = Vec3::new(0.0, 0.0, 2.0);
        assert!(!hits(ORIGIN, Quat::IDENTITY, behind, Quat::IDENTITY));
    }

    #[test]
    fn respects_rect_rotation_quirk() {
        // Rect yawed 60 degrees; ray at x = 0.3 meets the rect plane 0.3 / cos(60) = 0.6 m
        // along the rect's X axis, outside its 0.5 m half-width -> false. The same ray hits
        // the unrotated rect (0.3 < 0.5).
        let origin = Vec3::new(0.3, 0.0, 0.0);
        assert!(hits(origin, Quat::IDENTITY, RECT_CENTER, Quat::IDENTITY));
        assert!(!hits(origin, Quat::IDENTITY, RECT_CENTER, yaw(PI / 3.0)));

        // Known quirk: rect yawed 90 degrees (edge-on) with the ray along its center line ->
        // true. Float error tilts the plane slightly, so the ray "hits" it at exactly the rect
        // center.
        assert!(hits(ORIGIN, Quat::IDENTITY, RECT_CENTER, yaw(PI / 2.0)));
    }
}
