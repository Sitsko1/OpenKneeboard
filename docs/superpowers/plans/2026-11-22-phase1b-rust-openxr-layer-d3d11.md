# Phase 1b: Rust OpenXR Layer, D3D11 (Matching Behavior) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a Rust OpenXR API layer (`okb-xr-layer`, 64-bit and 32-bit) that appends the same quads and draws the same pixels as the C++ layer for D3D11 games, selectable instead of the C++ layer through an app setting.

**Architecture:**
- **`okb-vr` (new, no `unsafe`):** ports of `VRKneeboard` (poses, recenter, gaze), `RayIntersectsRect` and `SpriteAtlas` (MaxRects packing).
- **`okb-xr-layer` (new `cdylib`):**
  - the loader negotiation, `xrGetInstanceProcAddr` hooks and session lifecycle;
  - a pure `compose` module (layer selection, truncation, topmost swap);
  - a D3D11 boundary (SHM texture mapping, sprite batch, renderer) that reuses `okb-shm::Reader`.
  - D3D12 and Vulkan sessions pass straight through; they come in Phases 1c and 1d.
- **Proof of parity:**
  - `okb-ffi` exposes `okb-vr` to C++, so C++ tests compare it with `VRKneeboard` and `SpriteAtlas`.
  - A C++ harness loads each layer DLL against a fake OpenXR runtime on a D3D11 device. It compares the appended quads and the rendered swapchain pixels.
- **Switch:** the product build now builds Rust: Corrosion builds both bitnesses into `bin/`, next to their own layer JSONs. An `AppSettings` field picks which JSON the OpenXR helper registers.

**Tech Stack:**
- Rust 1.99.1, `windows-sys`, plus `windows` (COM) and `openxr-sys` (new; see Global Constraints).
- `fxc` (Windows SDK) for the DXBC sprite shaders.
- Corrosion 0.6.1, CMake + MSVC v143, GoogleTest.

**Spec:** `docs/superpowers/specs/2026-11-22-rust-port-plugins-passthrough-design.md` (4.1 "C++/Rust layer switch" and "32-bit", 5 Phase 1, 6). **Definition of done:** `docs/internals/rust-port-parity-contract.md` ("OpenXR layer" unit of parity, "Layer recorded-frame comparison"). Phase 1a plan, for the conventions this plan reuses: `docs/superpowers/plans/2026-11-22-phase1a-rust-shm.md`.

## Global Constraints

- **No behavior changes**, including quirks (spec 5: "porting and behavior changes are separate steps"). Where C++ terminates the game (`fatal()`, a throwing `check_hresult`, an out-of-range `.at()`), Rust terminates too: every exported entry point runs its body under `catch_unwind` and calls `std::process::abort()` on a panic. Never unwind into the loader. No-crash behavior is Phase 2.
- **Layer quads** must match the parity contract: count and order, `pose` and `size` within 1e-5, `subImage`, `layerFlags`, `eyeVisibility` and `space`. Swapchain pixels must match C++ within ±1 per 8-bit channel. This is a plan decision: the contract governs quads only, and pixels prove the renderer.
- **`unsafe`** only in the OpenXR/D3D boundary modules of `okb-xr-layer`: `lib.rs`, `next.rs`, `kneeboard.rs`, `d3d11/*`, each block with `// SAFETY:` (spec 6). `okb-vr` and `okb-xr-layer/src/compose.rs` are `#![forbid(unsafe_code)]`.
- **Dependencies:** the 1a list plus two that are new, both in `okb-xr-layer` only:
  - `windows = "0.62"`, D3D11/DXGI COM;
  - `openxr-sys = "0.11"`, OpenXR C types and loader structs. If it lacks `XrNegotiateLoaderInfo`/`XrNegotiateApiLayerRequest`/`XrApiLayerCreateInfo`, define those four `#[repr(C)]` structs in `lib.rs` from `loader_interfaces.h`.

  No math crate: `okb-vr` hand-ports the DirectXMath/SimpleMath operations it needs, in `f32`, in DirectXMath's operation order.
- **Both bitnesses:** the layer builds for `x86_64` and `i686` (spec 4.1). On `i686`, the export must be the undecorated `OpenKneeboard_xrNegotiateLoaderApiLayerInterface`, so use a `.def` file, as C++ does.
- **Layer name and identity** come from CMake through Corrosion env vars, with `option_env!` defaults for standalone cargo:
  - `OKB_OPENXR_LAYER_NAME` (default `XR_APILAYER_FREDEMMOTT_OpenKneeboard`)
  - `OKB_OPENXR_LAYER_DESCRIPTION`
  - `OKB_OPENXR_LAYER_IMPL_VERSION`
  - `OKB_REVERSE_DOMAIN` (default `com.openkneeboard`)
  - `OKB_VERSION_MAJOR`/`MINOR`/`PATCH`/`BUILD` (defaults `1`/`13`/`0`/`0`), used for the SHM `Names`.
- **Static CRT:** `+crt-static`, as in 1a (`rust/.cargo/config.toml`).
- `cargo clippy --all-targets` and `cargo fmt --check` are clean; CI builds both targets.
- US English in docs and comments.

## Review Focus

1. **A D3D12 or Vulkan game with the Rust layer selected:** no kneeboard, and the game must run as if no layer were present. That means no crash, `xrEndFrame` forwarded untouched, and the `XR_KHR_vulkan_enable2` functions not hooked. Pinned by `d3d12_session_passes_through` and `vulkan_functions_not_hooked` (Task 3).
2. **A 32-bit D3D11 game:** the loader looks up the layer's entry point by its JSON name, so a decorated `_name@12` export means the layer silently never loads. Pinned by the `/DEF` export plus the Task 6 `dumpbin` check on both DLLs.
3. **Both layers registered at once** (switch flipped; installer upgrade re-enabling the C++ JSON): the loader would load two layers with the same name. Enabling one implementation must disable the other's JSON, and "disable" must disable both. Pinned by the `OpenXRLayerRegistry.*` tests (Task 9).
4. **App closed or no feeder:** the game's frame must reach the runtime exactly as submitted (same `layers` pointer and `layerCount`). Pinned by the `NoFeederPassThrough` and `ZeroGameLayersPassThrough` parity scripts (Task 8).
5. **Atlas size change mid-session** (layer added, texture resized): the old swapchain is destroyed and its resources released before the new one is used, and the quads keep matching. Pinned by the `AtlasResize` parity script (Task 8).

---

### Task 1: `okb-vr` math, `ray_intersects_rect` and `VrKneeboard`

**Files:**
- Create: `rust/crates/okb-vr/Cargo.toml`, `src/lib.rs`, `src/math.rs`, `src/ray.rs`, `src/vr_kneeboard.rs`
- Modify: `rust/crates/okb-ffi/src/lib.rs`, `rust/crates/okb-ffi/Cargo.toml`, `rust/crates/okb-ffi/include/okb_ffi.h` (regenerated)
- Test: in-crate tests mirroring `src/tests/VRKneeboard_test.cpp` and `src/tests/RayIntersectsRect_test.cpp`; `src/tests/VRKneeboardParity_test.cpp` (C++), added to `src/tests/CMakeLists.txt`

**Interfaces:**
- Consumes: `okb_shm::wire::{Config, LayerConfig}` (field names as in `wire.rs`: `vr.pose.{x,eye_y,z,rx,ry,rz}`, `vr.physical_size`, `vr.zoom_scale`, `vr.gaze_target_scale.{horizontal,vertical}`, `vr.opacity.{normal,gaze}`, `vr.enable_gaze_zoom`, `vr_enabled`, `layer_id`; `Config.vr.{recenter_count, force_zoom, enable_gaze_input_focus}`, `global_input_layer_id`).
- Produces:
  ```rust
  pub struct Vec2 { pub x: f32, pub y: f32 }
  pub struct Vec3 { pub x: f32, pub y: f32, pub z: f32 }
  pub struct Quat { pub x: f32, pub y: f32, pub z: f32, pub w: f32 }
  pub struct Pose { pub position: Vec3, pub orientation: Quat }      // Default = zero position, (0,0,0,0) quat, like C++ `Pose {}`
  pub fn ray_intersects_rect(ray_origin: Vec3, ray_orientation: Quat, rect_center: Vec3, rect_orientation: Quat, rect_size: Vec2) -> bool;
  pub struct RenderParameters { pub kneeboard_pose: Pose, pub kneeboard_size: Vec2, pub kneeboard_opacity: f32, pub is_looking_at_kneeboard: bool }
  pub struct VrLayer<'a> { pub config: &'a LayerConfig, pub params: RenderParameters }
  /// What C++ passes to `SHM::ActiveConsumers::SetActiveInGameViewID`, if it calls it.
  pub enum ActiveViewUpdate { NoCall, Set(u64) }                       // Set(0) == C++ `SetActiveInGameViewID({})`
  pub struct VrKneeboard { /* recenter_count, recenter matrix, eye_height: Option<f32>, is_looking: HashMap<u64,bool> */ }
  impl VrKneeboard {
      pub fn new() -> Self;
      pub fn get_layers<'a>(&mut self, config: &Config, layers: &'a [LayerConfig], hmd: Pose) -> (Vec<VrLayer<'a>>, ActiveViewUpdate);
  }
  ```
  The `okb-ffi` additions (cbindgen) are `okb_vr_kneeboard_new/free`, `okb_vr_kneeboard_get_layers(kb, const OkbConfig*, const OkbLayerConfig*, count, OkbPose hmd, OkbVrLayerOut* out /*16*/, uint32_t* out_count, uint64_t* active_view /*UINT64_MAX = NoCall*/)` and `okb_ray_intersects_rect(...)`. `OkbVrLayerOut` holds `layer_index` (into the input slice) plus the `RenderParameters` fields.

- [ ] **Step 1: Write failing Rust tests** mirroring every `TEST` in `VRKneeboard_test.cpp` (`pose_is_stable_without_recenter`, `recenter_follows_headset_translation_in_xz`, `recenter_uses_yaw_only`, `recenter_captures_eye_height`, `head_movement_without_recenter_does_not_move_kneeboard`, `first_frame_does_not_recenter`) and `RayIntersectsRect_test.cpp` (`hits_center_straight_ahead`, `misses_when_pointing_away`, `hits_inside_edge`, `misses_outside_edge`, `misses_when_rect_behind_origin`, `respects_rect_rotation_quirk`). Use the same inputs, expected values and epsilon (1e-5). Add `gaze_focus_prefers_global_input_layer_while_looking` (expects `Set(global_id)`), `gaze_focus_falls_back_to_last_looked_at_layer` (reverse order, skips the global layer), `gaze_focus_disabled_makes_no_call` (`NoCall`), and `non_vr_layers_are_skipped`.
- [ ] **Step 2: Run** `cargo nextest run -p okb-vr` and confirm it fails to compile (no crate).
- [ ] **Step 3: Implement `okb-vr`.**
  - `math.rs` holds a row-major `Matrix` (DirectXMath row-vector convention) with exactly the operations `VRKneeboard.cpp` and `RayIntersectsRect.cpp` use:
    - `rotation_x/y/z`, `translation`, `mul`, `translation()`;
    - `Quat::from_rotation_matrix` (the `XMQuaternionRotationMatrix` algorithm) and `Quat::to_euler` (SimpleMath `ToEuler`);
    - `Vec3::rotate(q)` (`XMVector3Rotate`), `dot`;
    - the plane-from-point-normal and ray-plane intersection that SimpleMath `Ray::Intersects(Plane)` uses.
  - `vr_kneeboard.rs` follows `src/lib/VRKneeboard.cpp` function for function, including the `mEyeHeight` first-pose capture and the per-layer `mIsLookingAtKneeboard` map.
  - Then add the FFI functions and the `okb-vr` dependency to `okb-ffi`, and regenerate the header with the CI cbindgen command.
- [ ] **Step 4: Run** `cargo nextest run -p okb-vr` and `cargo nextest run -p okb-vr --target i686-pc-windows-msvc`; expect all to pass.
- [ ] **Step 5: Write `VRKneeboardParity_test.cpp`.** A table of scripted steps, each one `(Config, layers, hmd Pose)`, runs through both a fresh C++ `TestableVRKneeboard` and an `okb_vr_kneeboard`.
  - Assert per step: equal layer count and indices, pose within 1e-5 (quaternions compared up to sign), size within 1e-5, equal opacity and `is_looking`.
  - Also assert the same `ActiveConsumers` in-game view ID after each step: read the C++ result via `SHM::ActiveConsumers::Get()` and apply the Rust `active_view`.
  - Scripts: `NoRecenter`, `RecenterMidScript` (`recenter_count` 0→1→2 with yawed and pitched HMD), `GazeZoomAndOpacity`, `GazeFocusGlobalLayer`, `GazeFocusFallback`, `MixedVrEnabled`.
- [ ] **Step 6: Run** the C++ build of `OpenKneeboard-Tests` (Debug) and `ctest -R "VRKneeboard|RayIntersects"`; expect all to pass.
- [ ] **Step 7: Commit:** `feat(okb-vr): VRKneeboard and RayIntersectsRect in Rust with C++ parity tests`.

### Task 2: `okb-vr` `SpriteAtlas`

**Files:**
- Create: `rust/crates/okb-vr/src/sprite_atlas.rs`
- Modify: `rust/crates/okb-ffi/src/lib.rs`, `okb_ffi.h`
- Test: in-crate tests; `src/tests/SpriteAtlasParity_test.cpp`

**Interfaces:**
- Produces:
  ```rust
  pub struct SpriteAtlas { /* size: PixelSize, rects: Vec<PixelRect> */ }
  pub enum AtlasError { DoesNotFit }                       // C++ throws std::runtime_error
  impl SpriteAtlas {
      pub fn new(sprites: &[PixelSize]) -> Result<Self, AtlasError>;
      pub fn size(&self) -> PixelSize;
      pub fn rect(&self, index: usize) -> PixelRect;        // panics out of range, like C++ `.at()`
  }
  pub const MAX_VIEW_RENDER_SIZE: PixelSize = PixelSize { width: 2048, height: 2048 };
  pub fn scaled_to_fit(size: PixelSize, container: PixelSize) -> PixelSize;   // Geometry2D::ScaledToFit, ShrinkOrGrow, then Rounded
  ```
  The FFI additions are `okb_sprite_atlas_new(const OkbPixelSize*, count, OkbPixelSize* out_size, OkbPixelRect* out_rects) -> int32_t` (0 on success, -1 for `DoesNotFit`) and `okb_scaled_to_fit`.

- [ ] **Step 1: Write failing tests:**
  - `single_sprite_gets_padding_column_and_row` (one 100×50 sprite: rect at (0,0) 100×50, atlas 100×50);
  - `atlas_edge_is_at_least_64_and_power_of_two_before_trim`;
  - `grows_by_256_until_everything_fits`;
  - `does_not_fit_beyond_16384` (one 16384×16384 sprite → `DoesNotFit`);
  - `scaled_to_fit_rounds_like_cpp`.
- [ ] **Step 2: Run** `cargo nextest run -p okb-vr sprite_atlas`; expect it to fail.
- [ ] **Step 3: Implement.** Port `MaxRectsBinPack` (Jukka Jylänki's RectangleBinPack, public domain; source in vcpkg's `rectanglebinpack` port) with `RectBestShortSideFit` and no rotation: `Insert`, `ScoreRect`, `FindPositionForNewNodeBestShortSideFit`, `SplitFreeNode`, `PruneFreeList`, iterating in the same order so ties break identically. `SpriteAtlas::new` follows `src/lib/SpriteAtlas.cpp`: the +1 padding, `bit_ceil(max(maxDim+1, max(ceil(sqrt(area)), 64)))`, the 256 increment, `MaxDimension` 16384 (`D3D11_REQ_TEXTURE2D_U_OR_V_DIMENSION`), and the size equal to the max right and bottom.
- [ ] **Step 4: Run** the tests; expect them to pass.
- [ ] **Step 5: Write `SpriteAtlasParity_test.cpp`:**
  - 200 deterministic pseudo-random inputs (fixed-seed `std::mt19937`, 1–16 sprites, sizes 1–4096);
  - plus the edge inputs: all-equal sizes, one huge plus many tiny, 16 × 2048×2048.
  - Compare the C++ `SpriteAtlas` and `okb_sprite_atlas_new`: atlas size and every rect equal, and both throw or both return -1. Add `ScaledToFitParity` over the same sizes against `MaxViewRenderSize`.
- [ ] **Step 6: Run** `ctest -R SpriteAtlas`; expect it to pass.
- [ ] **Step 7: Commit:** `feat(okb-vr): SpriteAtlas (MaxRects BSSF) in Rust with C++ parity tests`.

### Task 3: `okb-xr-layer` loader skeleton (pass-through)

**Files:**
- Create: `rust/crates/okb-xr-layer/Cargo.toml` (`crate-type = ["cdylib", "rlib"]`; `rlib` so tests can call the exports), `build.rs`, `OpenKneeboard-OpenXR-Rust.def`, `src/lib.rs`, `src/next.rs`, `src/identity.rs`
- Test: in-crate tests with a Rust fake "next" (`src/test_support.rs`, `#[cfg(test)]`)

**Interfaces:**
- Produces:
  - Export `extern "system" fn OpenKneeboard_xrNegotiateLoaderApiLayerInterface(*const XrNegotiateLoaderInfo, *const c_char, *mut XrNegotiateApiLayerRequest) -> XrResult`.
  - Hooked through `xrGetInstanceProcAddr`: `xrCreateSession`, `xrDestroySession`, `xrDestroyInstance`, `xrEndFrame`, `xrEnumerateApiLayerProperties`, `xrEnumerateInstanceExtensionProperties`. Not hooked: `xrCreateVulkanInstanceKHR`/`xrCreateVulkanDeviceKHR`, which go to the next layer.
  - `pub(crate) struct Next`: the function pointers for the C++ `OPENKNEEBOARD_NEXT_OPENXR_FUNCS` list minus the Vulkan ones, plus `get_instance_proc_addr`. Loaded once in `xrCreateApiLayerInstance`.
  - `pub(crate) static STATE: Mutex<Option<LayerState>>`, where `LayerState { next: Next, runtime_name: String, kneeboard: Option<Kneeboard> }`. `Kneeboard` is a placeholder in this task: `xrEndFrame` always forwards.
  - `identity.rs`: `LAYER_NAME`, `LAYER_DESCRIPTION`, `LAYER_IMPL_VERSION: u32`, `shm_names() -> okb_shm::Names` (from the Global Constraints env vars).

- [ ] **Step 1: Write failing tests:**
  - `negotiate_rejects_other_layer_name` (`XR_ERROR_INITIALIZATION_FAILED`);
  - `negotiate_fills_request` (interface version 1, `XR_CURRENT_API_VERSION`, both function pointers set);
  - `enumerate_api_layer_properties_reports_one_layer` (capacity 0 → count 1 and `XR_SUCCESS`; wrong `type` → `XR_ERROR_VALIDATION_FAILURE`);
  - `enumerate_extensions_for_own_layer_is_empty`;
  - `create_instance_chains_to_next` (strips one `nextInfo` level);
  - `d3d12_session_passes_through` (no kneeboard; `xrEndFrame` forwards the same pointer and count);
  - `vulkan_functions_not_hooked` (`xrGetInstanceProcAddr("xrCreateVulkanDeviceKHR")` returns the fake next's pointer);
  - `second_session_returns_limit_reached` (C++ `XR_ERROR_LIMIT_REACHED` when a kneeboard exists).
- [ ] **Step 2: Run** `cargo nextest run -p okb-xr-layer`; expect it to fail (crate missing).
- [ ] **Step 3: Implement** following `src/injectables/OpenXRKneeboard.cpp:474-910`.
  - `xrCreateSession`:
    1. read `xrGetInstanceProperties` into `runtime_name`;
    2. forward to next;
    3. on success, walk the `next` chain for `XR_TYPE_GRAPHICS_BINDING_D3D11_KHR` with a non-null device, and record a pending D3D11 session (the kneeboard itself comes in Task 8).
    4. For any other binding, log and do nothing.
  - Logging: `OutputDebugStringW` with prefix `OpenKneeboard-OpenXR-Rust`; wording is free.
  - Wrap every exported/hooked fn body in `std::panic::catch_unwind(..).unwrap_or_else(|_| std::process::abort())`.
  - `build.rs` passes `/DEF:OpenKneeboard-OpenXR-Rust.def` with `cargo:rustc-cdylib-link-arg`.
- [ ] **Step 4: Run** the tests on both targets, and `cargo build -p okb-xr-layer --target i686-pc-windows-msvc`. Then run `dumpbin /exports` (VS dev shell) on both `okb_xr_layer.dll`; expect exactly `OpenKneeboard_xrNegotiateLoaderApiLayerInterface`, undecorated.
- [ ] **Step 5: Commit:** `feat(okb-xr-layer): OpenXR loader negotiation and pass-through hooks`.

### Task 4: `compose`: layer selection and quad order (pure)

**Files:**
- Create: `rust/crates/okb-xr-layer/src/compose.rs`
- Test: in-module tests

**Interfaces:**
- Consumes: `okb_vr::{VrLayer, SpriteAtlas, scaled_to_fit, MAX_VIEW_RENDER_SIZE, Pose, Vec2}`, `okb_shm::wire::{Config, LayerConfig, PixelRect}`.
- Produces:
  ```rust
  pub struct LayerSprite { pub source_rect: PixelRect, pub dest_rect: PixelRect, pub opacity: f32 }
  pub struct QuadSpec { pub pose: Pose, pub size: Vec2, pub image_rect: PixelRect }
  /// `None` = forward the game's frame unchanged (C++ "No active layers").
  pub fn compose(config: &Config, vr_layers: &[VrLayer], atlas: &SpriteAtlas, max_layer_count: u32, game_layer_count: u32)
      -> Option<(Vec<QuadSpec>, Vec<LayerSprite>)>;
  /// C++ `GetSpriteAtlas` cache key and inputs. Sizes come from *all* frame layers, not only VR-enabled ones (quirk).
  pub fn atlas_key(config: &Config, upscale: bool) -> u64;
  pub fn atlas_sizes(layers: &[LayerConfig], upscale: bool) -> Vec<PixelSize>;
  pub fn is_upscaling_enabled(config: &Config, runtime_name: &str) -> bool; // Automatic => runtime_name.starts_with("Varjo")
  ```

- [ ] **Step 1: Write failing tests:**
  - `truncates_to_runtime_max_minus_game_layers` (3 VR layers, max 4, game 2 → 2 quads);
  - `zero_room_returns_none`;
  - `global_input_layer_swapped_to_last_quad` (sprites keep SHM order; quads 0 and last swapped, each keeping its own `image_rect`);
  - `dest_rect_is_atlas_rect_by_vr_index` (quirk: with a non-VR layer first, VR layer 0 uses `atlas.rect(0)`);
  - `atlas_key_ignores_layer_sizes_and_flips_without_upscale` (`!key` when not upscaling);
  - `upscaling_automatic_only_on_varjo`, `upscaling_always_on_and_off`;
  - `game_layers_above_max_panics` (C++ unsigned underflow then crash; `#[should_panic]`).
- [ ] **Step 2: Run** them; expect them to fail.
- [ ] **Step 3: Implement** from `OpenXRKneeboard.cpp:234-340` and `:439-460`. The truncation formula is `if vr + game <= max { vr } else { max - game }`, with `checked_sub(..).expect(..)` for the underflow.
- [ ] **Step 4: Run** them; expect them to pass.
- [ ] **Step 5: Commit:** `feat(okb-xr-layer): pure quad composition matching the C++ layer`.

### Task 5: D3D11 boundary: SHM texture mapping, sprite batch, renderer

**Files:**
- Create: `rust/crates/okb-xr-layer/src/d3d11/mod.rs`, `shm_client.rs`, `state.rs`, `sprite_batch.rs`, `renderer.rs`; shader compile in `build.rs`
- Test: in-module tests on a WARP device (`D3D11CreateDevice` with `D3D_DRIVER_TYPE_WARP`)

**Interfaces:**
- Consumes: `okb_shm::{Reader, Frame, ConsumerKind}`, `compose::LayerSprite`.
- Produces:
  ```rust
  pub struct MappedFrame<'a> { pub config: &'a Config, pub srv: &'a ID3D11ShaderResourceView, pub fence: &'a ID3D11Fence, pub fence_in: u64 }
  pub struct ShmClient { /* reader: Reader<Box<dyn FnMut()>>, session_changed: Rc<Cell<bool>>, frames: [CachedResources; 2] */ }
  impl ShmClient {
      pub fn new(device: &ID3D11Device) -> Option<Self>;           // LUID via IDXGIDevice→adapter desc; ConsumerKind::OpenXrD3d11
      pub fn reader(&mut self) -> &mut Reader<..>;
      pub fn map(&mut self, frame: &Frame) -> MappedFrame<'_>;     // C++ SHM::D3D11::Reader::Map; panics on HRESULT failure
  }
  pub struct DeviceContextState;                                 // C++ D3D11::DeviceContextState + ScopedDeviceContextStateChange
  impl DeviceContextState { pub fn scoped<'a>(&'a mut self, ctx: &'a ID3D11DeviceContext1) -> impl Drop + 'a; }
  pub struct SwapchainImage { pub texture: ID3D11Texture2D, pub rtv: ID3D11RenderTargetView }
  pub struct Renderer;                                           // owns SpriteBatch + ID3D11DeviceContext4
  impl Renderer {
      pub fn new(device: &ID3D11Device) -> Self;
      pub fn render_layers(&mut self, dest: &SwapchainImage, dest_size: PixelSize, frame: &MappedFrame, sprites: &[LayerSprite]); // Wait(fence, fence_in), ClearAndRender
  }
  ```

- [ ] **Step 1: Write failing WARP tests:**
  - `sprite_copies_source_rect_to_dest_rect` (a 4×4 source with distinct premultiplied colors, drawn 1:1 into an 8×8 target at offset (2,2); read back; exact texels inside, cleared (0,0,0,0) outside);
  - `opacity_and_tint_scale_premultiplied_color` (tint (1,1,1,1), opacity 0.5 → each channel halved ±1);
  - `blend_is_premultiplied_over` (two overlapping sprites);
  - `map_reuses_resources_until_handle_changes` (same handle → same SRV pointer; new handle → new SRV);
  - `session_change_drops_cached_resources`;
  - `context_state_is_restored` (a VS bound before `render_layers` is still bound after).
- [ ] **Step 2: Run** `cargo nextest run -p okb-xr-layer d3d11`; expect it to fail.
- [ ] **Step 3: Implement.**
  - **Shaders:** `build.rs` compiles `../../../src/shaders/Sprite.hlsl` with `fxc /nologo /T vs_5_0 /E SpriteVertexShader /Fo $OUT_DIR/sprite_vs.dxbc`, and the same with `ps_5_0` and `SpritePixelShader`. It finds `fxc` from `OKB_FXC` (set by CMake), else the newest `%ProgramFiles(x86)%\Windows Kits\10\bin\<ver>\x64\fxc.exe`, and panics with that message if neither exists. The shaders are embedded with `include_bytes!`.
  - **Sprite batch:** port `src/lib/D3D11/SpriteBatch.cpp`: the input layout, the uniform buffer `{source dims, dest dims}`, and six vertices per sprite with tex-clamp half-texel insets.
  - **Pipeline state:** from DirectXTK `CommonStates`:
    - `LinearClamp`: `MIN_MAG_MIP_LINEAR`, clamp on all axes, `MaxAnisotropy` 16, `ComparisonFunc` NEVER, `MaxLOD` `f32::MAX`;
    - `CullCounterClockwise`: SOLID, CULL_BACK, depth clip on, multisample on;
    - `DepthNone`;
    - `AlphaBlend`: ONE / INV_SRC_ALPHA for color and alpha, ADD, write all;
    - blend factor white, sample mask `0xFFFFFFFF`.
  - **Renderer:** `render_layers` follows `src/lib/D3D11/Renderer.cpp`: `Wait`, `Begin`, `Clear`, one `Draw` per sprite with `tint * opacity`, then `End`.
  - **Mapping:** `ShmClient::map` follows `src/lib/SHM/D3D11.cpp`, with `OpenSharedResource1` and `OpenSharedFence`.
- [ ] **Step 4: Run** the tests on x64 and i686; expect them to pass.
- [ ] **Step 5: Commit:** `feat(okb-xr-layer): D3D11 SHM mapping, sprite batch and renderer`.

### Task 6: Product build: both layer DLLs, JSONs and CI

**Files:**
- Modify: `third-party/CMakeLists.txt` (include `corrosion.cmake` unconditionally), `third-party/corrosion.cmake`, `src/injectables/CMakeLists.txt`, `src/lib/RuntimeFiles-app.cpp.in`, `src/lib/include/OpenKneeboard/RuntimeFiles.hpp`, `.github/workflows/ci.yml`
- Create: `src/injectables/OpenKneeboard-OpenXR-Rust.in.json`
- Test: `src/tests/OpenXRLayerLoad_test.cpp`

**Interfaces:**
- Produces:
  - Output files in `${BUILD_OUT_BINDIR}`: `OpenKneeboard-OpenXR-Rust64.dll` and `OpenKneeboard-OpenXR-Rust.json` (64-bit build); `OpenKneeboard-OpenXR-Rust32.dll` and `OpenKneeboard-OpenXR-Rust32.json` (32-bit sub-build). Each JSON has the same `name` as the C++ layer, its own `library_path`, the same `functions` entry, and the same `disable_environment`.
  - `RuntimeFiles::OPENXR_64BIT_RUST_JSON`, `OPENXR_32BIT_RUST_JSON`, `OPENXR_64BIT_RUST_DLL`, `OPENXR_32BIT_RUST_DLL`.
  - Compile definitions on `OpenKneeboard-Tests`: `OKB_TEST_CPP_LAYER_DLL` (`$<TARGET_FILE:OpenKneeboard-OpenXR>`) and `OKB_TEST_RUST_LAYER_DLL`.

- [ ] **Step 1: Write the failing test `OpenXRLayerLoad.*`**, parameterized over both DLL paths: `LoadLibraryW` succeeds; `GetProcAddress("OpenKneeboard_xrNegotiateLoaderApiLayerInterface")` is non-null; negotiating with the right name returns `XR_SUCCESS` and with `"XR_APILAYER_wrong"` returns `XR_ERROR_INITIALIZATION_FAILED`.
- [ ] **Step 2: Build** with `-DOPENKNEEBOARD_BUILD_TESTS=ON`; the test fails because the Rust DLL path is not defined yet.
- [ ] **Step 3: Implement.**
  - `corrosion_import_crate(... CRATES okb-ffi okb-xr-layer)`. For the 32-bit sub-build (`BUILD_IS_32BIT`), set `Rust_CARGO_TARGET i686-pc-windows-msvc` before import. Keep `okb-ffi` test-only (`EXCLUDE_FROM_ALL` unless tests).
  - `corrosion_set_env_vars(okb_xr_layer ...)` passes the Global Constraints identity variables and `OKB_FXC=${FXC_EXE}`.
  - A post-build copy places the DLL and PDB under the names above, and `file(GENERATE)` writes the JSON, both mirroring how `OpenKneeboard-OpenXR` does it. Wire it into `build32` so the 64-bit build produces both, as `DUALARCH` does.
  - The installer already harvests `bin/*.*`, so nothing changes there.
  - CI: the build job already installs the toolchain (and its `i686` target). Add a step after the build that runs `dumpbin /exports` on both Rust DLLs and fails unless the undecorated export is present. Use `ilammy/msvc-dev-cmd` pinned to a SHA, or `vswhere` to find `dumpbin`.
- [ ] **Step 4: Run** a default configure **without** tests and build; expect both Rust DLLs and JSONs in `out/<config>/bin`. Then build with tests and run `ctest -R OpenXRLayerLoad`; expect it to pass for both DLLs.
- [ ] **Step 5: Commit:** `build: ship the Rust OpenXR layer (64/32-bit) next to the C++ layer`.

### Task 7: C++ layer harness: fake runtime and D3D11 feeder (C++ layer only)

**Files:**
- Create: `src/tests/OpenXRLayerHarness.hpp`, `src/tests/OpenXRLayerHarness.cpp`, `src/tests/OpenXRLayerParity_test.cpp`
- Modify: `src/tests/CMakeLists.txt`

**Interfaces:**
- Consumes: `OKB_TEST_CPP_LAYER_DLL` (Task 6); C++ `SHM::Writer`.
- Produces (used by Task 8):
  ```cpp
  struct HarnessStep {
    SHM::Config mConfig; std::vector<SHM::LayerConfig> mLayers; // empty + mDetach = writer detached
    bool mDetach {false};
    XrPosef mHmdPose; XrTime mDisplayTime; uint32_t mGameLayerCount {1};
  };
  struct HarnessScript {
    std::string mRuntimeName {"FakeRuntime"}; uint32_t mMaxLayerCount {16};
    std::vector<int64_t> mSwapchainFormats {DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB};
    PixelSize mSourceTextureSize {1024, 1024}; std::vector<HarnessStep> mSteps;
  };
  struct RecordedQuad { XrPosef mPose; XrExtent2Df mSize; int mSwapchainOrdinal; XrRect2Di mImageRect;
                        XrCompositionLayerFlags mFlags; XrEyeVisibility mEye; std::string mSpace; /* "LOCAL"/"VIEW"/"OTHER" */ };
  struct RecordedFrame { bool mGameLayersForwardedUnchanged; uint32_t mLayerCount; std::vector<RecordedQuad> mQuads;
                         std::vector<uint8_t> mSwapchainPixels; /* BGRA/RGBA as created, tightly packed */ uint64_t mActiveInGameViewID; };
  std::vector<RecordedFrame> RunHarness(const std::filesystem::path& layerDll, const HarnessScript&);
  ```

- [ ] **Step 1: Probe the adapter.** Create the device (hardware adapter, else WARP). Check that a BGRA8 texture with `D3D11_RESOURCE_MISC_SHARED | SHARED_NTHANDLE` and a shared `ID3D11Fence` can be created and reopened through `OpenSharedResource1`/`OpenSharedFence`.
  - If neither adapter supports this, `RunHarness` callers `GTEST_SKIP()` with `"no D3D11 adapter supports shared NT-handle textures and fences"`. Report this in your status (it decides whether CI runs the parity tests).
- [ ] **Step 2: Write failing characterization tests**, run against the C++ DLL only:
  - `CppLayer.SingleLayerAppendsOneQuad` (1 quad, flags `BLEND_TEXTURE_SOURCE_ALPHA_BIT`, eye BOTH, space LOCAL, image rect = the atlas rect, and non-zero pixels in it);
  - `CppLayer.TruncatesToRuntimeMax` (max 3, game 2, SHM 3 → 1 quad);
  - `CppLayer.NoFeederPassesThrough` (`mGameLayersForwardedUnchanged` true, 0 quads).
- [ ] **Step 3: Implement the harness.**
  - **Feeder:** an in-process `SHM::Writer` with the device's LUID, publishing each step's config and layers. The texture is filled with a deterministic pattern (texel (x,y) = premultiplied `(x%251, y%241, (x+y)%239, 255)`), and the fence is signaled to step+1.
  - **Fake runtime:** `nextGetInstanceProcAddr` returns fakes for every function in `OPENKNEEBOARD_NEXT_OPENXR_FUNCS` that the layer calls.
    - `xrGetSystemProperties` uses the script's `maxLayerCount` and `xrGetInstanceProperties` its runtime name.
    - `xrCreateReferenceSpace` records LOCAL/VIEW by handle, and `xrLocateSpace` returns the step's HMD pose with both valid flags.
    - Swapchains are real textures on the device in the first format the layer picks.
    - `xrEndFrame` records the layers. After `xrReleaseSwapchainImage`, the released image is copied to staging for the pixels.
  - **Driver:** `LoadLibraryW`, negotiate, `createApiLayerInstance` with a fake `XrApiLayerNextInfo`, then `xrCreateSession` with `XrGraphicsBindingD3D11KHR`. Run the steps, then `xrDestroySession`, `xrDestroyInstance` and `FreeLibrary`.
- [ ] **Step 4: Run** `ctest -R CppLayer`; expect it to pass (or skip with the probe message).
- [ ] **Step 5: Commit:** `test: OpenXR layer harness with a fake runtime and D3D11 feeder`.

### Task 8: Rust `xrEndFrame` and layer parity

**Files:**
- Create: `rust/crates/okb-xr-layer/src/kneeboard.rs`
- Modify: `rust/crates/okb-xr-layer/src/lib.rs`, `src/tests/OpenXRLayerParity_test.cpp`

**Interfaces:**
- Consumes: Tasks 1–5 (`VrKneeboard`, `SpriteAtlas`, `compose`, `ShmClient`, `Renderer`, `DeviceContextState`) and `RunHarness` (Task 7).
- Produces:
  ```rust
  pub(crate) struct Kneeboard { /* next, max_layer_count, local_space, view_space, is_varjo, swapchain, swapchain_size,
                                   images: Vec<SwapchainImage>, atlas_key, atlas, vr: VrKneeboard, shm: ShmClient, renderer, state */ }
  impl Kneeboard {
      pub(crate) fn new(next: &Next, instance: XrInstance, system: XrSystemId, session: XrSession, runtime_name: &str, binding: &XrGraphicsBindingD3D11KHR) -> Self;
      pub(crate) fn end_frame(&mut self, session: XrSession, info: *const XrFrameEndInfo) -> XrResult;
  }
  impl Drop for Kneeboard { /* destroy spaces and swapchain, like the C++ destructor */ }
  ```

- [ ] **Step 1: Write failing parity tests** `RustLayerParity.<Script>`, each calling `ExpectParity(script)`. That helper runs `RunHarness` with both DLLs and asserts, frame by frame:
  - equal `mGameLayersForwardedUnchanged`, `mLayerCount` and quad count;
  - per quad: pose within 1e-5 (orientation up to sign) and size within 1e-5;
  - equal `mSwapchainOrdinal`, `mImageRect`, `mFlags`, `mEye` and `mSpace`;
  - pixels within ±1 per channel, and equal `mActiveInGameViewID`.

  Scripts:
  - `SingleLayer`, `MultiLayer` (3);
  - `MoreLayersThanRuntimeAllows` (max 4, game 2, SHM 3);
  - `GlobalInputLayerNotLast`;
  - `NonVrLayerFirst` (the atlas-index quirk);
  - `GazeZoomAndOpacity` (HMD aimed at layer 2; gaze focus on);
  - `RecenterMidScript`;
  - `NoFeederPassThrough` (`mDetach` step), `ZeroGameLayersPassThrough` (`mGameLayerCount` 0);
  - `AtlasResize` (layer added at step 3; source size changed at step 5);
  - `VarjoUpscaling` (runtime `"Varjo OpenXR"`, a 3000×1500 layer);
  - `FormatFallback` (runtime offers only `DXGI_FORMAT_R8G8B8A8_UNORM_SRGB`);
  - `SameDisplayTimeUsesCachedPose` (two steps with equal `mDisplayTime` and different poses).
- [ ] **Step 2: Run** `ctest -R RustLayerParity`; expect it to fail (the Rust layer only passes through).
- [ ] **Step 3: Implement `Kneeboard`**, line for line from `OpenXRKneeboard.cpp:130-460` and `OpenXRD3D11Kneeboard.cpp`.
  - Format preference: `B8G8R8A8_UNORM_SRGB`→RTV `B8G8R8A8_UNORM`, then `R8G8B8A8_UNORM_SRGB`→`R8G8B8A8_UNORM`, else the first format for both.
  - `VrKneeboard::get_layers` returns `ActiveViewUpdate`; on `Set(id)` call `okb_shm::active_consumers::set_active_in_game_view_id(&names, id)`, and on `NoCall` do nothing.
  - The HMD pose cache is **process-static** (`displayTime` key, initially 0), like the C++ function statics.
  - Swapchain creation failure → panic (C++ `fatal`).
  - The appended `XrCompositionLayerQuad` array and the layer-pointer vector live in `Kneeboard` until the next call, as in C++.
  - `xrCreateSession` (Task 3) now constructs the `Kneeboard` for D3D11; `xrDestroySession`/`xrDestroyInstance` drop it.
- [ ] **Step 4: Run** `ctest -R "RustLayerParity|CppLayer|OpenXRLayerLoad"` and `cargo nextest run -p okb-xr-layer` (x64 and i686); expect all to pass.
- [ ] **Step 5: Commit:** `feat(okb-xr-layer): D3D11 xrEndFrame matching the C++ layer, with recorded-frame parity`.

### Task 9: Layer switch, docs and the manual hardware test

**Files:**
- Create: `src/lib/include/OpenKneeboard/OpenXRLayerRegistry.hpp`, `src/lib/OpenXRLayerRegistry.cpp`, `src/tests/OpenXRLayerRegistry_test.cpp`, `docs/internals/rust-openxr-layer.md`
- Modify:
  - `src/app/app-common/include/OpenKneeboard/AppSettings.hpp`, `AppSettings.cpp`
  - `src/app/app-common/include/OpenKneeboard/OpenXRMode.hpp`, `OpenXRMode.cpp`
  - `src/utilities/openxr-helper.cpp`
  - `src/app/app-winui3/VRSettingsPage.xaml.cpp` (compiled by CI only; the UWP toolset is missing locally)
  - `docs/internals/rust-port-parity-contract.md`

**Interfaces:**
- Produces:
  ```cpp
  enum class OpenXRLayerImplementation { Cpp, Rust };            // AppSettings::mOpenXRLayer, default Cpp; JSON "OpenXRLayer": "Cpp" | "Rust"
  // OpenXRLayerRegistry.hpp — pure, testable
  /// Registry values to set to 1 (disabled) when enabling `chosenJsonPath`.
  std::vector<std::wstring> LayersToDisableOnEnable(std::span<const std::wstring> registeredPaths,
                                                    std::wstring_view chosenJsonPath,
                                                    std::span<const std::wstring_view> okbJsonFileNames /* all four JSON names */);
  /// Registry values to set to 1 when disabling OpenXR for one bitness: every registered path ending in either implementation's JSON name for that bitness.
  std::vector<std::wstring> LayersToDisable(std::span<const std::wstring> registeredPaths, std::span<const std::wstring_view> bitnessJsonFileNames);
  task<void> SetOpenXR64ModeWithHelperProcess(OpenXRMode, OpenXRLayerImplementation);
  task<void> SetOpenXR32ModeWithHelperProcess(OpenXRMode, OpenXRLayerImplementation);
  ```
  Helper commands: `enable-HKLM-64`, `enable-HKLM-64-rust`, `enable-HKLM-32`, `enable-HKLM-32-rust`, `disable-HKLM-64` and `disable-HKLM-32`. A disable command disables both implementations' JSONs.

- [ ] **Step 1: Write failing tests:**
  - `OpenXRLayerRegistry.EnablingRustDisablesCpp`, `EnablingCppDisablesRust`;
  - `EnablingKeepsUnrelatedLayers` (a non-OpenKneeboard JSON is untouched);
  - `EnablingDisablesStaleCopiesOfSameJson` (the existing `ends_with` behavior);
  - `DisableCoversBothImplementations`;
  - `AppSettings.OpenXRLayerRoundTrips` (missing field → `Cpp`; `"Rust"` round-trips; unknown string → `Cpp`).
- [ ] **Step 2: Run** them; expect them to fail.
- [ ] **Step 3: Implement.**
  - `openxr-helper.cpp` calls the two pure functions.
  - `OpenXRMode.cpp` maps `(mode, impl)` to a command.
  - `VRSettingsPage`: `OpenXR64Enabled()`/`OpenXR32Enabled()` return true if either implementation's JSON is enabled, and the toggles pass `mKneeboard`'s `AppSettings().mOpenXRLayer`.
  - Serialize the enum the way existing settings enums are serialized.
- [ ] **Step 4: Run** `ctest -R "OpenXRLayerRegistry|AppSettings"` and the full `ctest`; expect them to pass. Push and confirm CI builds `app-winui3`.
- [ ] **Step 5: Write the docs.**
  - **`docs/internals/rust-openxr-layer.md`:**
    - how to select the Rust layer: set `"OpenXRLayer": "Rust"` in the settings JSON, then turn OpenXR off and on in VR settings;
    - the installer re-registers the C++ layer on upgrade;
    - D3D11 only until Phases 1c/1d;
    - a **manual hardware checklist**: `hello_xr -g D3D11` on the active runtime with each layer, kneeboard visible at the same place and size, gaze zoom, recenter, a second view as global input layer, the app closed mid-session (kneeboard disappears, no crash), and one 64-bit D3D11 game.
  - **Parity contract:**
    - add "Phase 1b scope: D3D11; D3D12/Vulkan sessions pass through in the Rust layer";
    - add to Known quirks: atlas sizes from all frame layers indexed by VR-layer index; atlas cache key ignores layer sizes; process-static HMD pose cache keyed on `displayTime` (a `displayTime` of 0 returns the default pose);
    - add the ±1 pixel rule under "How parity is measured".
- [ ] **Step 6: Commit:** `feat: app setting selects the C++ or Rust OpenXR layer; docs and manual test`.
- [ ] **Step 7: Hand the manual checklist to your human partner** and record their pass/fail in the ledger. Phase 1b is done when it passes.
