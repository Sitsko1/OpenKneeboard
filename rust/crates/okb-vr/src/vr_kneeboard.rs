//! Port of `src/lib/VRKneeboard.cpp`, function for function.
use std::collections::HashMap;

use okb_shm::wire::{Config, LayerConfig, VrRenderSettings};

use crate::math::{Matrix, Pose, Quat, Vec2, Vec3};
use crate::ray::ray_intersects_rect;

/// C++ `VRKneeboard::RenderParameters`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderParameters {
    pub kneeboard_pose: Pose,
    pub kneeboard_size: Vec2,
    pub kneeboard_opacity: f32,
    pub is_looking_at_kneeboard: bool,
}

/// C++ `VRKneeboard::Layer`.
#[derive(Clone, Copy, Debug)]
pub struct VrLayer<'a> {
    pub config: &'a LayerConfig,
    pub params: RenderParameters,
}

/// What C++ passes to `SHM::ActiveConsumers::SetActiveInGameViewID`, if it calls it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveViewUpdate {
    NoCall,
    /// `Set(0)` is C++ `SetActiveInGameViewID({})`.
    Set(u64),
}

/// C++ `VRKneeboard::Sizes`.
struct Sizes {
    normal: Vec2,
    zoomed: Vec2,
}

/// C++ `VRKneeboard`: per-session kneeboard placement state.
#[derive(Debug)]
pub struct VrKneeboard {
    recenter_count: u64,
    recenter: Matrix,
    is_looking_at_kneeboard: HashMap<u64, bool>,
    eye_height: Option<f32>,
}

impl Default for VrKneeboard {
    fn default() -> Self {
        Self::new()
    }
}

impl VrKneeboard {
    pub fn new() -> Self {
        Self {
            recenter_count: 0,
            recenter: Matrix::IDENTITY,
            is_looking_at_kneeboard: HashMap::new(),
            eye_height: None,
        }
    }

    /// C++ `GetLayers`; the `SetActiveInGameViewID` call it would make is returned instead.
    pub fn get_layers<'a>(
        &mut self,
        config: &Config,
        layers: &'a [LayerConfig],
        hmd: Pose,
    ) -> (Vec<VrLayer<'a>>, ActiveViewUpdate) {
        self.eye_height.get_or_insert(hmd.position.y);

        let ret: Vec<VrLayer<'a>> = layers
            .iter()
            .filter(|l| l.vr_enabled.get())
            .map(|l| VrLayer {
                config: l,
                params: self.get_render_parameters(config, l, hmd),
            })
            .collect();

        if !config.vr.enable_gaze_input_focus.get() {
            return (ret, ActiveViewUpdate::NoCall);
        }
        let global_input_layer_id = config.global_input_layer_id;

        // Keep the current view for as long as we're looking at it, rather than potentially
        // alternating between overlapping views
        if let Some(input) = ret
            .iter()
            .find(|l| l.config.layer_id == global_input_layer_id)
            && input.params.is_looking_at_kneeboard
        {
            // Set it even if there's "no change", to cover active vs input views
            let id = input.config.layer_id;
            return (ret, ActiveViewUpdate::Set(id));
        }

        let looked_at = ret.iter().rev().find(|l| {
            l.params.is_looking_at_kneeboard && l.config.layer_id != global_input_layer_id
        });
        let update = ActiveViewUpdate::Set(looked_at.map_or(0, |l| l.config.layer_id));
        (ret, update)
    }

    fn get_kneeboard_pose(
        &mut self,
        vr: &VrRenderSettings,
        layer: &LayerConfig,
        hmd: Pose,
    ) -> Pose {
        self.eye_height.get_or_insert(hmd.position.y);
        let pose = &layer.vr.pose;
        self.maybe_recenter(vr, hmd);
        let eye_height = self.eye_height.expect("set above");
        let matrix = Matrix::rotation_x(pose.rx)
            .mul(&Matrix::rotation_y(pose.ry))
            .mul(&Matrix::rotation_z(pose.rz))
            .mul(&Matrix::translation(Vec3::new(
                pose.x,
                pose.eye_y + eye_height,
                pose.z,
            )))
            .mul(&self.recenter);

        Pose {
            position: matrix.get_translation(),
            orientation: Quat::from_rotation_matrix(&matrix),
        }
    }

    fn get_kneeboard_size(
        &self,
        config: &Config,
        layer: &LayerConfig,
        is_looking_at_kneeboard: bool,
    ) -> Vec2 {
        let sizes = self.get_sizes(&config.vr, layer);

        if config.vr.force_zoom.get()
            || (is_looking_at_kneeboard && layer.vr.enable_gaze_zoom.get())
        {
            sizes.zoomed
        } else {
            sizes.normal
        }
    }

    fn get_sizes(&self, _vr: &VrRenderSettings, layer: &LayerConfig) -> Sizes {
        let physical_size = layer.vr.physical_size;
        let virtual_width = physical_size.width;
        let virtual_height = physical_size.height;

        Sizes {
            normal: Vec2::new(virtual_width, virtual_height),
            zoomed: Vec2::new(
                virtual_width * layer.vr.zoom_scale,
                virtual_height * layer.vr.zoom_scale,
            ),
        }
    }

    fn maybe_recenter(&mut self, vr: &VrRenderSettings, hmd: Pose) {
        if vr.recenter_count == self.recenter_count {
            return;
        }
        self.recenter(vr, hmd);
    }

    fn recenter(&mut self, vr: &VrRenderSettings, hmd: Pose) {
        let mut pos = hmd.position;
        self.eye_height = Some(pos.y);
        pos.y = 0.0;

        // We're only going to respect ry (yaw) as we want the new center to remain
        // gravity-aligned
        let quat = hmd.orientation;

        self.recenter = Matrix::rotation_y(quat.to_euler().y)
            .mul(&Matrix::translation(Vec3::new(pos.x, pos.y, pos.z)));

        self.recenter_count = vr.recenter_count;
    }

    fn get_render_parameters(
        &mut self,
        config: &Config,
        layer: &LayerConfig,
        hmd: Pose,
    ) -> RenderParameters {
        let kneeboard_pose = self.get_kneeboard_pose(&config.vr, layer, hmd);
        let is_looking_at_kneeboard =
            self.is_looking_at_kneeboard(config, layer, hmd, kneeboard_pose);

        RenderParameters {
            kneeboard_pose,
            kneeboard_size: self.get_kneeboard_size(config, layer, is_looking_at_kneeboard),
            kneeboard_opacity: if is_looking_at_kneeboard {
                layer.vr.opacity.gaze
            } else {
                layer.vr.opacity.normal
            },
            is_looking_at_kneeboard,
        }
    }

    fn is_looking_at_kneeboard(
        &mut self,
        config: &Config,
        layer: &LayerConfig,
        hmd: Pose,
        kneeboard_pose: Pose,
    ) -> bool {
        let sizes = self.get_sizes(&config.vr, layer);
        // C++ `operator[]`: inserts `false` even when returning early below, which keeps any
        // earlier value unchanged.
        let is_looking = self
            .is_looking_at_kneeboard
            .entry(layer.layer_id)
            .or_insert(false);

        // C++ compares the float against the double 0.1.
        let scale = layer.vr.gaze_target_scale;
        if f64::from(scale.horizontal) < 0.1 || f64::from(scale.vertical) < 0.1 {
            return false;
        }

        let mut current_size = if *is_looking {
            sizes.zoomed
        } else {
            sizes.normal
        };

        current_size.x *= scale.horizontal;
        current_size.y *= scale.vertical;

        *is_looking = ray_intersects_rect(
            hmd.position,
            hmd.orientation,
            kneeboard_pose.position,
            kneeboard_pose.orientation,
            current_size,
        );

        *is_looking
    }
}

#[cfg(test)]
mod tests {
    //! Mirrors `src/tests/VRKneeboard_test.cpp`, plus gaze-focus and `vr_enabled` coverage.
    use std::f32::consts::PI;

    use okb_shm::wire::{CBool, Config, GazeTargetScale, LayerConfig, SizeF32, VrPose};

    use super::*;

    const EPSILON: f32 = 1e-5;
    const DEGREES: f32 = PI / 180.0;

    struct Fixture {
        config: Config,
        layer: LayerConfig,
    }

    impl Fixture {
        fn new() -> Self {
            let mut config = Config::default();
            config.vr.enable_gaze_input_focus = CBool::FALSE;
            let mut layer = LayerConfig {
                layer_id: 1,
                vr_enabled: CBool::TRUE,
                ..LayerConfig::default()
            };
            layer.vr.pose = VrPose::default();
            layer.vr.physical_size = SizeF32 {
                width: 0.2,
                height: 0.3,
            };
            Self { config, layer }
        }

        fn get(&self, kb: &mut VrKneeboard, hmd: Pose) -> Pose {
            let layers = std::slice::from_ref(&self.layer);
            let (out, _) = kb.get_layers(&self.config, layers, hmd);
            assert_eq!(out.len(), 1);
            out[0].params.kneeboard_pose
        }

        /// Bump `recenter_count` and recenter at `hmd`.
        fn recenter_at(&mut self, kb: &mut VrKneeboard, hmd: Pose) -> Pose {
            self.config.vr.recenter_count += 1;
            self.get(kb, hmd)
        }
    }

    fn hmd(position: Vec3, yaw_deg: f32, pitch_deg: f32, roll_deg: f32) -> Pose {
        Pose {
            position,
            orientation: Quat::from_yaw_pitch_roll(
                yaw_deg * DEGREES,
                pitch_deg * DEGREES,
                roll_deg * DEGREES,
            ),
        }
    }

    fn at(x: f32, y: f32, z: f32) -> Pose {
        hmd(Vec3::new(x, y, z), 0.0, 0.0, 0.0)
    }

    fn assert_near_vec(a: Vec3, b: Vec3) {
        assert!((a.x - b.x).abs() <= EPSILON, "{a:?} vs {b:?}");
        assert!((a.y - b.y).abs() <= EPSILON, "{a:?} vs {b:?}");
        assert!((a.z - b.z).abs() <= EPSILON, "{a:?} vs {b:?}");
    }

    fn assert_near(a: Pose, b: Pose) {
        assert_near_vec(a.position, b.position);
        // q and -q are the same rotation
        let o = (a.orientation, b.orientation);
        let dot = (o.0.x * o.1.x + o.0.y * o.1.y + o.0.z * o.1.z + o.0.w * o.1.w).abs();
        assert!((dot - 1.0).abs() <= EPSILON, "{a:?} vs {b:?}");
    }

    #[test]
    fn pose_is_stable_without_recenter() {
        let f = Fixture::new();
        let mut kb = VrKneeboard::new();
        let a = f.get(&mut kb, at(0.0, 1.7, 0.0));
        let b = f.get(&mut kb, at(0.0, 1.7, 0.0));
        assert_eq!(a, b);
    }

    #[test]
    fn recenter_follows_headset_translation_in_xz() {
        let mut f = Fixture::new();
        let mut kb = VrKneeboard::new();
        let a = f.recenter_at(&mut kb, at(0.0, 1.7, 0.0));
        let b = f.recenter_at(&mut kb, at(1.0, 1.7, 2.0));
        let delta = Vec3::new(
            b.position.x - a.position.x,
            b.position.y - a.position.y,
            b.position.z - a.position.z,
        );
        assert_near_vec(delta, Vec3::new(1.0, 0.0, 2.0));
    }

    #[test]
    fn recenter_uses_yaw_only() {
        // A tilted head must not tilt kneeboards.
        let mut f = Fixture::new();
        let mut yaw_only = VrKneeboard::new();
        let mut tilted = VrKneeboard::new();
        let a = f.recenter_at(&mut yaw_only, hmd(Vec3::new(0.0, 1.7, 0.0), 90.0, 0.0, 0.0));
        f.config.vr.recenter_count -= 1;
        let b = f.recenter_at(&mut tilted, hmd(Vec3::new(0.0, 1.7, 0.0), 90.0, 30.0, 20.0));
        assert_near(a, b);
    }

    #[test]
    fn recenter_captures_eye_height() {
        let mut f = Fixture::new();
        let mut kb = VrKneeboard::new();
        let low = f.recenter_at(&mut kb, at(0.0, 1.2, 0.0));
        let high = f.recenter_at(&mut kb, at(0.0, 1.8, 0.0));
        assert!((high.position.y - low.position.y - 0.6).abs() <= EPSILON);
    }

    #[test]
    fn head_movement_without_recenter_does_not_move_kneeboard() {
        let f = Fixture::new();
        let mut kb = VrKneeboard::new();
        let a = f.get(&mut kb, at(0.0, 1.7, 0.0));
        let b = f.get(&mut kb, at(0.5, 1.7, 0.0));
        assert_near(a, b);
    }

    #[test]
    fn first_frame_does_not_recenter() {
        // Known quirk in the parity contract: no recenter until `recenter_count` changes from 0.
        let f = Fixture::new();
        let mut at_origin = VrKneeboard::new();
        let mut offset = VrKneeboard::new();
        let a = f.get(&mut at_origin, at(0.0, 1.7, 0.0));
        let b = f.get(&mut offset, hmd(Vec3::new(1.0, 1.7, 2.0), 45.0, 0.0, 0.0));
        assert_near(a, b);
    }

    /// A 1 m x 1 m layer 1 m straight ahead of an HMD at (0, 1.7, 0) looking down -Z.
    fn gaze_layer(layer_id: u64, looking: bool) -> LayerConfig {
        let mut layer = LayerConfig {
            layer_id,
            vr_enabled: CBool::TRUE,
            ..LayerConfig::default()
        };
        layer.vr.pose = VrPose {
            x: 0.0,
            eye_y: 0.0,
            z: -1.0,
            rx: 0.0,
            ry: 0.0,
            rz: 0.0,
        };
        layer.vr.physical_size = SizeF32 {
            width: 1.0,
            height: 1.0,
        };
        if !looking {
            // Below 0.1: never a gaze target.
            layer.vr.gaze_target_scale = GazeTargetScale {
                vertical: 0.0,
                horizontal: 0.0,
            };
        }
        layer
    }

    fn gaze_config(global_input_layer_id: u64) -> Config {
        Config {
            global_input_layer_id,
            ..Config::default()
        }
    }

    fn looking_ids(layers: &[VrLayer]) -> Vec<(u64, bool)> {
        layers
            .iter()
            .map(|l| (l.config.layer_id, l.params.is_looking_at_kneeboard))
            .collect()
    }

    #[test]
    fn gaze_focus_prefers_global_input_layer_while_looking() {
        let layers = [gaze_layer(1, true), gaze_layer(2, true)];
        let mut kb = VrKneeboard::new();
        let (out, update) = kb.get_layers(&gaze_config(1), &layers, at(0.0, 1.7, 0.0));
        assert_eq!(looking_ids(&out), [(1, true), (2, true)]);
        assert_eq!(update, ActiveViewUpdate::Set(1));
    }

    #[test]
    fn gaze_focus_falls_back_to_last_looked_at_layer() {
        // The first layer with the global ID is not looked at; the last layer is, but has the
        // global ID too, so it is skipped and the reverse scan settles on layer 2.
        let layers = [
            gaze_layer(1, true),
            gaze_layer(3, false),
            gaze_layer(2, true),
            gaze_layer(3, true),
        ];
        let mut kb = VrKneeboard::new();
        let (out, update) = kb.get_layers(&gaze_config(3), &layers, at(0.0, 1.7, 0.0));
        assert_eq!(
            looking_ids(&out),
            [(1, true), (3, false), (2, true), (3, true)]
        );
        assert_eq!(update, ActiveViewUpdate::Set(2));
    }

    #[test]
    fn gaze_focus_clears_when_nothing_is_looked_at() {
        let layers = [gaze_layer(1, false), gaze_layer(2, false)];
        let mut kb = VrKneeboard::new();
        let (_, update) = kb.get_layers(&gaze_config(1), &layers, at(0.0, 1.7, 0.0));
        assert_eq!(update, ActiveViewUpdate::Set(0));
    }

    #[test]
    fn gaze_focus_disabled_makes_no_call() {
        let layers = [gaze_layer(1, true)];
        let mut config = gaze_config(1);
        config.vr.enable_gaze_input_focus = CBool::FALSE;
        let mut kb = VrKneeboard::new();
        let (out, update) = kb.get_layers(&config, &layers, at(0.0, 1.7, 0.0));
        assert_eq!(looking_ids(&out), [(1, true)]);
        assert_eq!(update, ActiveViewUpdate::NoCall);
    }

    #[test]
    fn non_vr_layers_are_skipped() {
        let mut layers = [
            gaze_layer(1, true),
            gaze_layer(2, true),
            gaze_layer(3, true),
        ];
        layers[1].vr_enabled = CBool::FALSE;
        let mut kb = VrKneeboard::new();
        let (out, _) = kb.get_layers(&gaze_config(0), &layers, at(0.0, 1.7, 0.0));
        let ids: Vec<u64> = out.iter().map(|l| l.config.layer_id).collect();
        assert_eq!(ids, [1, 3]);
        assert!(std::ptr::eq(out[1].config, &layers[2]));
    }
}
