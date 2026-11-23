//! Pure layer selection and quad ordering, ported from the layer handling in
//! `OpenXRKneeboard::xrEndFrame` and `GetSpriteAtlas`.
#![forbid(unsafe_code)]

use okb_shm::wire::{Config, LayerConfig, PixelRect, PixelSize, Upscaling};
use okb_vr::{MAX_VIEW_RENDER_SIZE, Pose, SpriteAtlas, Vec2, VrLayer, scaled_to_fit};

/// C++ `SHM::LayerSprite`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerSprite {
    pub source_rect: PixelRect,
    pub dest_rect: PixelRect,
    pub opacity: f32,
}

/// The variable parts of an `XrCompositionLayerQuad`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuadSpec {
    pub pose: Pose,
    pub size: Vec2,
    pub image_rect: PixelRect,
}

/// `None` means forward the game's frame unchanged (C++ "No active layers").
///
/// Panics if `game_layer_count` exceeds `max_layer_count` while the layers do
/// not fit: C++ underflows an unsigned subtraction and then crashes.
pub fn compose(
    config: &Config,
    vr_layers: &[VrLayer],
    atlas: &SpriteAtlas,
    max_layer_count: u32,
    game_layer_count: u32,
) -> Option<(Vec<QuadSpec>, Vec<LayerSprite>)> {
    let (vr, game, max) = (
        vr_layers.len(),
        game_layer_count as usize,
        max_layer_count as usize,
    );
    let count = if vr + game <= max {
        vr
    } else {
        max.checked_sub(game)
            .expect("game layer count exceeds the runtime's maximum")
    };
    if count == 0 {
        return None;
    }

    let mut top_most = count - 1;
    let mut quads = Vec::with_capacity(count);
    let mut sprites = Vec::with_capacity(count);
    for (i, layer) in vr_layers[..count].iter().enumerate() {
        // Quirk kept from C++: the atlas is indexed by VR layer index, while
        // its sprites came from all frame layers.
        let dest_rect = atlas.rect(i);
        sprites.push(LayerSprite {
            source_rect: layer.config.vr.location_on_texture,
            dest_rect,
            opacity: layer.params.kneeboard_opacity,
        });
        if layer.config.layer_id == config.global_input_layer_id {
            top_most = i;
        }
        quads.push(QuadSpec {
            pose: layer.params.kneeboard_pose,
            size: layer.params.kneeboard_size,
            image_rect: dest_rect,
        });
    }
    quads.swap(top_most, count - 1);
    Some((quads, sprites))
}

/// C++ `GetSpriteAtlas` cache key: the texture size, inverted when not upscaling.
pub fn atlas_key(config: &Config, upscale: bool) -> u64 {
    let raw = (u64::from(config.texture_size.width) << 32) | u64::from(config.texture_size.height);
    if upscale { raw } else { !raw }
}

/// C++ `GetSpriteAtlas` inputs. Quirk: from all frame layers, not only VR-enabled ones.
pub fn atlas_sizes(layers: &[LayerConfig], upscale: bool) -> Vec<PixelSize> {
    layers
        .iter()
        .map(|l| {
            let size = l.vr.location_on_texture.size;
            if upscale {
                scaled_to_fit(size, MAX_VIEW_RENDER_SIZE)
            } else {
                size
            }
        })
        .collect()
}

pub fn is_upscaling_enabled(config: &Config, runtime_name: &str) -> bool {
    match config.vr.quirks.openxr_upscaling {
        Upscaling::AUTOMATIC => runtime_name.starts_with("Varjo"),
        Upscaling::ALWAYS_ON => true,
        Upscaling::ALWAYS_OFF => false,
        // C++ `std::unreachable()`; unknown values come from a peer process.
        other => unreachable!("invalid upscaling value {}", other.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use okb_vr::RenderParameters;

    fn sz(width: u32, height: u32) -> PixelSize {
        PixelSize { width, height }
    }

    fn layer(id: u64, w: u32, h: u32) -> LayerConfig {
        let mut l = LayerConfig {
            layer_id: id,
            ..Default::default()
        };
        l.vr.location_on_texture.size = sz(w, h);
        l
    }

    fn params(x: f32) -> RenderParameters {
        let mut pose = Pose::default();
        pose.position.x = x;
        RenderParameters {
            kneeboard_pose: pose,
            kneeboard_size: Vec2 { x, y: 1.0 },
            kneeboard_opacity: x / 10.0,
            is_looking_at_kneeboard: false,
        }
    }

    fn vr<'a>(cfgs: &'a [LayerConfig]) -> Vec<VrLayer<'a>> {
        cfgs.iter()
            .enumerate()
            .map(|(i, config)| VrLayer {
                config,
                params: params(i as f32 + 1.0),
            })
            .collect()
    }

    fn atlas(n: usize) -> SpriteAtlas {
        SpriteAtlas::new(&vec![sz(100, 50); n]).unwrap()
    }

    #[test]
    fn truncates_to_runtime_max_minus_game_layers() {
        let cfgs = [layer(1, 10, 10), layer(2, 10, 10), layer(3, 10, 10)];
        let (quads, sprites) = compose(&Config::default(), &vr(&cfgs), &atlas(3), 4, 2).unwrap();
        assert_eq!((quads.len(), sprites.len()), (2, 2));
        // All fit: no truncation.
        let (quads, _) = compose(&Config::default(), &vr(&cfgs), &atlas(3), 8, 2).unwrap();
        assert_eq!(quads.len(), 3);
    }

    #[test]
    fn zero_room_returns_none() {
        let cfgs = [layer(1, 10, 10)];
        assert!(compose(&Config::default(), &vr(&cfgs), &atlas(1), 2, 2).is_none());
        assert!(compose(&Config::default(), &[], &atlas(1), 4, 0).is_none());
    }

    #[test]
    fn global_input_layer_swapped_to_last_quad() {
        let cfgs = [layer(1, 10, 10), layer(2, 10, 10), layer(3, 10, 10)];
        let config = Config {
            global_input_layer_id: 1,
            ..Default::default()
        };
        let a = atlas(3);
        let (quads, sprites) = compose(&config, &vr(&cfgs), &a, 8, 0).unwrap();
        assert_eq!(quads[0].image_rect, a.rect(2));
        assert_eq!(quads[0].size.x, 3.0);
        assert_eq!(quads[2].image_rect, a.rect(0));
        assert_eq!(quads[2].size.x, 1.0);
        assert_eq!(quads[1].image_rect, a.rect(1));
        // Sprites keep SHM order.
        for (i, s) in sprites.iter().enumerate() {
            assert_eq!(s.dest_rect, a.rect(i));
            assert_eq!(s.opacity, (i as f32 + 1.0) / 10.0);
        }
    }

    #[test]
    fn dest_rect_is_atlas_rect_by_vr_index() {
        // Frame layers: [non-VR, VR]; the atlas was built from both.
        let all = [layer(1, 10, 10), layer(2, 20, 20)];
        let a = SpriteAtlas::new(&atlas_sizes(&all, false)).unwrap();
        let vr_only = [all[1]];
        let (quads, sprites) = compose(&Config::default(), &vr(&vr_only), &a, 4, 0).unwrap();
        assert_eq!(sprites[0].dest_rect, a.rect(0));
        assert_eq!(quads[0].image_rect, a.rect(0));
        assert_eq!(sprites[0].source_rect.size, sz(20, 20));
    }

    #[test]
    fn atlas_key_ignores_layer_sizes_and_flips_without_upscale() {
        let config = Config {
            texture_size: sz(3, 5),
            ..Default::default()
        };
        let on = atlas_key(&config, true);
        assert_eq!(on, (3u64 << 32) | 5);
        assert_eq!(atlas_key(&config, false), !on);
        let layers = [layer(1, 1000, 500)];
        assert_eq!(atlas_sizes(&layers, false), vec![sz(1000, 500)]);
        assert_eq!(atlas_sizes(&layers, true), vec![sz(2048, 1024)]);
    }

    #[test]
    fn upscaling_automatic_only_on_varjo() {
        let config = Config::default();
        assert!(is_upscaling_enabled(&config, "Varjo OpenXR Runtime"));
        assert!(!is_upscaling_enabled(&config, "SteamVR/OpenXR"));
        assert!(!is_upscaling_enabled(&config, "My Varjo"));
    }

    #[test]
    fn upscaling_always_on_and_off() {
        let mut config = Config::default();
        config.vr.quirks.openxr_upscaling = Upscaling::ALWAYS_ON;
        assert!(is_upscaling_enabled(&config, "SteamVR/OpenXR"));
        config.vr.quirks.openxr_upscaling = Upscaling::ALWAYS_OFF;
        assert!(!is_upscaling_enabled(&config, "Varjo OpenXR Runtime"));
    }

    #[test]
    #[should_panic(expected = "exceeds")]
    fn game_layers_above_max_panics() {
        let cfgs = [layer(1, 10, 10)];
        compose(&Config::default(), &vr(&cfgs), &atlas(1), 2, 3);
    }
}
