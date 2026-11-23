---
parent: Internals
---

# Rust Port Parity Contract (Phase 1: SHM and the OpenXR layer)

Phase 1 ports the SHM frame transport (`src/lib/SHM.cpp`) and the OpenXR API layer (`src/injectables/OpenXR*`) to Rust **without changing behavior**. This document defines "without changing behavior": Phase 1 is done when the Rust code meets every rule below, and the evidence comes from the harness described in "How parity is measured".

Behavior changes, including the isolation fixes the spec calls for, belong to Phase 2 and are listed at the end. Spec: `docs/superpowers/specs/2026-11-22-rust-port-plugins-passthrough-design.md`.

## Unit of parity

Parity is judged on two observable outputs. Internal structure, threading, and code layout are free to differ.

1. **SHM:**
   - the byte layout of the shared header and frame slots (`SharedData`, `SharedData::Frame`, `Config`, `LayerConfig`, `VRRenderSettings`, `VRLayer`), and the name of the mapping and mutex.
   - the byte layout and contents of the second mapping, `ActiveConsumers` (`src/lib/include/OpenKneeboard/SHM/ActiveConsumers.hpp`), after each reader and layer action. The app reads it to pick the render resolution, to enable the global mouse hook (`AnyVR()`), and to show the elevated-consumer warning.
   - for the same sequence of writer actions (`Writer(luid)`, `lock`/`unlock`, `BeginFrame`, `SubmitFrame`, `SubmitEmptyFrame`, `Detach`, destruction), the result of each `Reader::MaybeGet()`: either the `Frame` fields (`mConfig`, `mLayers`, `mFenceIn`, `mIndex`, and whether `mTexture`/`mFence` are usable) or the `Frame::Error` value. This also covers `Reader::operator bool` and `OnSessionChanged()` calls.
2. **OpenXR layer:** for each `xrEndFrame`, the list of `XrCompositionLayerQuad` the layer appends to the game's layers:
   - count and order
   - `pose` and `size`
   - `subImage` (swapchain and image rect)
   - `layerFlags` and `eyeVisibility`
   - the `space` the quads are expressed in

## Must match exactly

- **SHM byte layout:** size and field offsets of every shared structure, for both 64-bit and 32-bit builds. Today C++ asserts only the total (`static_assert(SHM_SIZE == 3024)` in `src/lib/SHM.cpp`). Phase 1 adds C++ `static_assert`s on field offsets, and Rust tests that assert the same sizes and offsets. Handles stay 64-bit wide (`FixedSizeHandle`) in both builds.
- **The constants `SHMSwapchainLength` (2) and `MaxViewCount` (16):** already enforced by `rust/crates/okb-shm/tests/matches_cpp.rs`.
- **Mapping and mutex names:** `{ProjectReverseDomain}/{Major}.{Minor}.{Patch}.{Build}-s{SHM_SIZE:x}` plus `.mutex`; ActiveConsumers is `{ProjectReverseDomain}/{Major}.{Minor}.{Patch}.{Build}/ActiveConsumers-s{size:x}`.
- **ActiveConsumers writes:** every `MaybeGet()` that passes the GPU check calls `ActiveConsumers::Set(consumerKind)` before the layer-count check, so `NoLayers` and successful reads both mark the consumer active; `NoFeeder` and `IncorrectGPU` do not. The `ActiveConsumers` field layout must match byte for byte.
- **`Frame::Error` results:** the same error for the same writer actions, *including precedence*. Checks run in this order: no feeder, then GPU mismatch, then no layers, then unusable handles.
- **Swapchain index selection:** `mFrameNumber % SHMSwapchainLength` for both reader and writer. `BeginFrame` returns `(mFrameNumber + 1) % SHMSwapchainLength`.
- **Session semantics:** a new `mSessionID` on writer construction and on `Detach()`. `OnSessionChanged()` fires on the first `MaybeGet()` that sees a new session.
- **Layer count:** one quad per VR-enabled `LayerConfig`, truncated to `mMaxLayerCount - frameEndInfo->layerCount` (the runtime's `maxLayerCount` minus the game's own layers; later layers are dropped). If that leaves zero, the game's `xrEndFrame` is forwarded unchanged.
- **Layer order:** SHM order, except that the quad of the global-input layer (`Config::mGlobalInputLayerID`) is swapped with the last quad, so it is drawn topmost (`std::swap(addedXRLayers.back(), addedXRLayers.at(topMost))` in `src/injectables/OpenXRKneeboard.cpp`). Only the quads are swapped; each keeps its own `subImage`.
- **Layer flags:** `XR_COMPOSITION_LAYER_BLEND_TEXTURE_SOURCE_ALPHA_BIT`.
- **Gaze-input focus side effects:** the same `SHM::ActiveConsumers::SetActiveInGameViewID` calls (see `VRKneeboard::GetLayers`).

## Acceptable differences

- **Pose:** position within 1e-5 m and orientation within 1e-5 rad of the C++ result (compare quaternions up to sign: `|dot(a, b)| ≥ 1 − 1e-5`). Quad `size` within 1e-5 m.
- **Logging and tracing:** wording, event names, and which events are emitted (`dprint`, TraceLogging) may differ.
- **Timing:** CPU time per call may differ, but it must not regress beyond the R4 budget (0.5 ms CPU in `xrEndFrame`).
- **Rust-internal error handling:** may replace C++ exceptions, *as long as the observable result is the same* (for example, a C++ path that throws and terminates must still terminate in Phase 1; changing that is Phase 2).
- **Out-of-range layer count:** The Rust reader clamps a peer-written `mLayerCount` above `MaxViewCount` (16) to 16 when copying layers; the C++ reader reads out of bounds there. A valid writer never exceeds 16 (the C++ writer `fatal()`s first).
- **Padding bytes:** Padding bytes have no defined value. The C++ writer copies a temporary whose trailing `Frame` padding is indeterminate (0xCC in Debug builds, stack contents in Release), so the differential harness masks the trailing padding of each `SharedData::Frame` slot along with `mSessionID`. The Rust writer always writes zeros there.

## How parity is measured

Both harnesses are built in Phase 1.

- **SHM differential harness:** feeds an identical script of writer actions to the C++ and the Rust writers (in-process, through `okb-ffi`) and reads back with both readers in all four writer/reader combinations. It compares every `MaybeGet()` result field by field, compares the bytes of the frame SHM after each step, and, for each consumer kind, which `ActiveConsumers` timestamp field a passing read sets (the timestamps themselves differ from run to run). The scripts start with the scenarios in `src/tests/SHM_test.cpp` and add multi-layer frames, swapchain wrap-around, and detach and reattach. The byte comparison masks `mSessionID`, which is random per writer and per detach, and the trailing padding of each frame slot (see "Acceptable differences"); every other byte must match. Implemented as `SHMParity.*` in `src/tests/SHMParity_test.cpp`.
- **Layer recorded-frame comparison:** implemented as `RustLayerParity.*` in `src/tests/OpenXRLayerParity_test.cpp`. A fake OpenXR runtime and a D3D11 feeder publish a fixed set of layers (including more layers than the runtime allows, and a global-input layer that is not last in SHM order), once through the C++ layer and once through the Rust layer. The harness records the appended quads per `xrEndFrame`; Phase 1 passes when every recorded frame matches under the rules above. It also compares the swapchain pixels within ±1 per 8-bit channel. It runs on WARP in CI. The C++/Rust layer switch (the `OpenXRLayer` app setting) is only needed for the manual hardware check in `rust-openxr-layer.md`.
- **Unit tests:** the C++ characterization tests in `src/tests/` (`VRKneeboard_test.cpp`, `RayIntersectsRect_test.cpp`, `SHM_test.cpp`) are mirrored in Rust with the same inputs and expected values.

## Phase 1b scope

The Rust layer supports D3D11 only. D3D12 and Vulkan sessions pass through with no kneeboard. This is temporary, until Phases 1c and 1d. Two observable differences follow from it:

- After a D3D12 or Vulkan session, a second `xrCreateSession` is accepted, where the C++ layer returns `XR_ERROR_LIMIT_REACHED`.
- `xrCreateVulkanInstanceKHR` and `xrCreateVulkanDeviceKHR` are forwarded to the next layer, not hooked.

## Known quirks (preserve in Phase 1)

Characterization tests found these behaviors. They may look wrong, but Phase 1 keeps them; fixing any of them is a separate, later change.

- `SemVer.ToSemVerStringCharacterization`: `ToSemVerString("1.2.3.4")` returns `"1.2.3.4"` unchanged, which is not valid SemVer. `CompareSemVer()` then logs a warning and returns `Equal` for any version it cannot parse.
- `RayIntersectsRect.RespectsRectRotation`: a ray along the center line of an edge-on rect (yawed 90°) reports a hit. Float error tilts the plane slightly, so the ray meets it at exactly the rect center. This feeds gaze detection (`VRKneeboard::IsLookingAtKneeboard`).
- `SHMTest.EmptyFrameCharacterization`: `Writer::SubmitEmptyFrame()` never sets `FEEDER_ATTACHED`, so readers see `NoFeeder` (not `NoLayers`) from a writer that has only submitted empty frames.
- `SHMTest.GpuMismatchCharacterization`: the GPU check comes before the layer-count check. A reader on the wrong GPU gets `IncorrectGPU` even for a zero-layer frame, while a reader on the right GPU gets `NoLayers`. A wrong-GPU reader also skips `ActiveConsumers::Set`.
- `VRKneeboard.FirstFrameDoesNotRecenter`: no recenter happens until `mRecenterCount` changes from 0. Before the first recenter, kneeboards are placed relative to the tracking-space origin, with eye height taken from the first headset pose ever seen.
- `SHM::Writer` (observed in code): constructing a writer unconditionally resets the whole shared header, even if another writer is live.
- `ActiveConsumers::Set(OpenXR_D3D12)` sets `mOpenXR_D3D11`, not `mOpenXR_D3D12`, so D3D12 consumers are reported as D3D11 and `mOpenXR_D3D12` is never set. Pinned by Rust test `d3d12_consumer_sets_d3d11_timestamp_quirk` and by `SHMParity.ActiveConsumers`.
- OpenXR layer sprite atlas: atlas sizes are computed from all frame layers but indexed by VR-layer index. The atlas cache key is the texture size only; it ignores the layer count and layer sizes. A frame that adds a layer without changing the texture size therefore indexes past its cached atlas and terminates the game (both layers). Pinned by the `AtlasResize` script, which changes the texture size.
- OpenXR layer HMD pose cache: it is process-static and keyed on `displayTime`. A `displayTime` of 0 returns the default pose without locating.

## Changes deferred to Phase 2

Phase 1 must **not** fix these; each is pinned by a test or listed in the spec, and Phase 2 changes it on purpose (spec Section 2.2 and rules R1–R5):

- **The blocking `INFINITE` wait:** `SHM::Reader::MaybeGet()` blocks on the cross-process mutex for as long as the writer holds it. Pinned by `SHMTest.ReaderBlocksWhileWriterHoldsLock`; Phase 2 inverts that test (lock-free reads, R1).
- **Unbounded GPU fence waits** queued in the game's D3D11/D3D12/Vulkan queues on values the app has not yet signaled (R1).
- **Crashes that cross into the game:** `fatal()`, `OPENKNEEBOARD_ASSERT`, and throwing `check_hresult` calls in code that runs in-game, both in the injectables and in the in-game `src/lib` code (`SHM*`, `D3D11/`, `D3D12/`, `Vulkan/`) (R2).
- **No watchdog:** a stale app keeps its last frame on screen indefinitely (R3).
- **No per-frame time budget** (R4), and pass-through for unknown runtime state (R5).
- **The lock-free v2 SHM wire format:** replaces the mutex-based v1 layout above. The contract's byte-layout rules apply to v1 only.
