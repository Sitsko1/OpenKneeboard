# OpenKneeboard: Rust core, plugin architecture, and VR passthrough masks

Status: draft for review · Date: 2026-11-22 · Branch: `Rusify-1`

## 1. Intent

Move OpenKneeboard to Rust **where performance and memory safety matter most**,
not everywhere. The overriding rule: **if OpenKneeboard crashes or bogs down,
the game it draws into must keep running smoothly.** On that foundation, add a
plugin architecture open to outside developers, then use it to ship a VR
passthrough-mask module with reference-mark auto-alignment.

Why (owner's words): performance and memory safety.

## 2. What OpenKneeboard is today (assessment)

Windows-only, ~65k lines of C++20. Shows reference material and notes in games
(primarily flight sims), in VR and on the desktop; driven by HOTAS/joystick
bindings or graphics tablets.

| Area | Path | Size | Role |
|---|---|---|---|
| App | `src/app/app-common`, `src/app/app-winui3` | ~43k lines | WinUI 3/XAML settings UI, tabs, page sources, profiles, input, CEF browser tabs, Lua/DCS integration, SteamVR overlay |
| Core lib | `src/lib` | ~15k lines | D3D11/D3D12/Vulkan rendering helpers, shared memory (SHM) frame transport, tracing, utilities |
| Injectables | `src/injectables` | ~2.6k lines | OpenXR API layer (D3D11/D3D12/Vulkan) and Detours window-capture hook; run **inside the game process** |
| API | `src/api` | ~0.3k lines | C API DLL, Lua API (remote control) |
| DCS hook | `src/dcs-hook` | ~0.3k lines | Lua export into DCS |
| Utilities | `src/utilities` | ~4.3k lines | Viewers, remote-control CLIs, trace tools, `capi-test`, `task-test` |

Feature inventory (from `docs/features`): PDFs, images, plain text, folders,
window capture, web dashboards (CEF), endless notebook (doodles), bookmarks,
DCS tabs (aircraft, briefing, mission, radio log, terrain), profiles, dual
kneeboards and multiple views, VR zoom, tint/brightness, remote controls
(Stream Deck, VoiceAttack, C/Lua APIs), graphics tablets (incl. OpenTabletDriver IPC).

Frame pipeline: the app renders into a shared GPU texture, then publishes it
through SHM (`src/lib/SHM.cpp`, swapchain length 2, max 16 views). Inside the
game, the OpenXR layer hooks `xrEndFrame` (`src/injectables/OpenXRKneeboard.cpp:202`),
reads the latest SHM frame, waits on the app's GPU fence, copies the frame into its own
swapchain, and appends quad layers to the game's frame.

Existing plugin system (experimental): a JSON manifest (`v1.json`, optionally
inside a `.OpenKneeboardPlugin` zip, or found via the registry) that defines web-based tab
types (`docs/api/web-dashboards/plugins.md`, `Plugin.hpp`, `PluginStore.cpp`).

### 2.1 Coverage baseline

**0%.** There are no automated tests: no CTest, no test framework, and CI only
builds. The only test-like code consists of two manual utilities (`capi-test`,
`task-test`). Behavior must be inferred from the docs and code, and characterized
by new tests before anything is ported (Phase 0).

### 2.2 Where the game is exposed today (the problems this design fixes)

1. **The game thread can block indefinitely on the app.** `SHM::Reader::MaybeGet()`
   takes a cross-process mutex with `WaitForSingleObject(..., INFINITE)`
   (`src/lib/SHM.cpp:217`) inside the game's `xrEndFrame`.
2. **The game's GPU queue can stall on the app.** The layer queues a GPU wait on
   a fence value the app promised to signal (`D3D11/Renderer.cpp:54`,
   `D3D12/Renderer.cpp:108`, plus a Vulkan timeline-semaphore wait). If the app dies first, the
   game's queue waits forever and risks a driver reset (TDR).
3. **App or runtime errors crash the game.** The injectables call `fatal()` (5 call sites:
   `OpenXRKneeboard.cpp:127,251`; `OpenXRVulkanKneeboard.cpp:89,208,244`) on OpenXR
   failures, deliberately terminating the host. The `src/lib` code that also runs
   in-game (`SHM*`, `D3D11/`, `D3D12/`, `Vulkan/`) adds about 27 more `fatal()`/
   `OPENKNEEBOARD_ASSERT` sites, plus throwing `check_hresult` calls (for example on the
   fence `Wait` calls in `src/lib/D3D11/Renderer.cpp:54` and `src/lib/D3D12/Renderer.cpp:108`).

Other in-app SHM readers: the SteamVR overlay (`SteamVRKneeboard`) and the
`src/utilities/viewer-*` tools also use `SHM::Reader`. The OpenXR layer also ships as a
32-bit build (`OpenKneeboard-OpenXR32.json`).

Already good: an abandoned mutex after an app crash is detected, and the shared state is reset.

## 3. Goals, non-goals, anti-goals

**Goals**
- G1. Game isolation: OpenKneeboard failures never freeze, stall, or crash the
  game. Worst case: the overlay disappears or shows a stale frame.
- G2. Rust for the in-game code and the frame transport first, then for other
  hot or unsafe paths, ranked by performance and safety value.
- G3. A plugin system open to outside developers, with a versioned contract
  and enforced safeguards.
- G4. A passthrough-mask module: define a 2D mask shape in a separate, out-of-game process.
- G5. Alignment: recognize printed target stickers or image-matched room features,
  position the mask in 3D like Recenter does (but from the references), and keep it
  aligned, with a re-scan frequency setting where the headset allows in-session scans.
- G6. A test harness and measured coverage, starting from today's 0%.

**Non-goals**
- Rewriting the WinUI 3 settings UI or the CEF browser tabs in Rust. They stay C++ in
  the app process unless a later decision says otherwise.
- Cross-platform support. This stays Windows-only.
- Native-code (DLL) third-party plugins.

**Anti-goals (failure even if it "works")**
- Any third-party code running inside the game process.
- Any new path where the game waits on the app without a bound.
- A port that cannot show it matches the old behavior (no parity evidence).
- Breaking existing users' settings, profiles, or existing web-tab plugins.

## 4. Architecture

```
┌──────────── Game process ────────────┐      ┌──────────── App process (C++ UI + Rust core) ───────┐
│ okb-xr-layer (Rust, OpenXR API layer)│      │ WinUI3 UI ─ C++ app-common ─┐                       │
│  - never blocks, never panics out    │◄─────┤ okb-core (Rust): SHM writer,│ plugin host (wasmtime)│
│  - draws quads + passthrough masks   │ SHM  │ frame scheduler, settings   │  ├ passthrough plugin │
│  - watchdog: drop overlay if stale   │(lock │ model (later)               │  └ 3rd-party plugins  │
└──────────────────────────────────────┘ free)└─────────────────────────────────────────────────────┘
                                               okb-mask-editor (Rust, 2D mask definition, out of game)
                                               okb-align-quest (Android, on-headset alignment companion)
```

### 4.1 Rust workspace (decided by owner: Rust, prioritized by perf/safety)

Cargo workspace at `rust/`, built by CMake through Corrosion, a CMake/Cargo integration
(the builder's call if a better integration exists). Planned crates:

- `okb-shm`: lock-free SHM frame transport (writer and reader); the wire format
  is versioned.
- `okb-xr-layer`: the OpenXR API layer DLL; replaces `src/injectables/OpenXR*`.
- `okb-plugin-host`: the wasmtime host, the WIT contract, permissions, and resource limits.
- `okb-passthrough`: mask model, alignment solver, and runtime capability probing (shared
  by the layer, the editor, and the plugin).
- `okb-mask-editor`: the out-of-game 2D mask definition tool.
- `okb-align-quest`: the on-headset Quest alignment companion (Android; Rust via
  `cargo-ndk`, or Kotlin if the camera API demands it, which is the builder's call).
- `okb-ffi`: C ABI to the C++ app (cbindgen-generated headers, or cxx).

The Detours window-capture hook stays C++ until Phase 7 (low payoff, high FFI cost).

**SHM migration (my call):** Phase 1's Rust port keeps the existing mutex-based
wire format byte-for-byte, so the C++ and Rust readers and writers can run side by side
and their output can be compared. Phase 2 introduces lock-free wire format v2; at that
point every reader (OpenXR layer, SteamVR overlay, viewers) moves to the Rust reader
through `okb-ffi`, and the writer publishes v2 only. There is no long-term dual format.

**C++/Rust layer switch (my call):** both layer DLLs ship during Phases 1–2. An
app setting chooses which DLL path the existing `OpenXRMode` registry logic
registers in the implicit-layer JSON. The C++ layer and the v1 SHM code are deleted
after Phase 2 passes hardware tests and one public release has shipped.

**32-bit (my call):** the Rust layer builds for both `x86_64` and `i686`, matching
today's kneeboard feature set. Passthrough masks are 64-bit only.

**Crash surface (binding):** R2 covers every `fatal()`, `OPENKNEEBOARD_ASSERT`, and
throwing `check_hresult` that can run in-game, not only the injectables' own calls.

### 4.2 Game-isolation rules (binding on all in-game code)

- R1. **No unbounded waits** on the game thread or the game's GPU queues. CPU: SHM
  reads are lock-free (seqlock or atomic index over the swapchain slots). GPU:
  only queue a wait on a fence value that the CPU has already seen completed
  (`GetCompletedValue() >= target`). Otherwise reuse the last frame we copied.
- R2. **No panics or `fatal()` cross into the host.** Every exported entry point is
  wrapped in `catch_unwind`; the release profile uses `panic = "abort"` only in
  the app, never in the layer. On any internal error, the layer passes the game's
  call straight through and disables itself for the session.
- R3. **Watchdog.** If the app's heartbeat (in the SHM header) goes stale for more than N ms
  (default 500), stop adding layers until it resumes.
- R4. **Per-frame time budget.** Layer work in `xrEndFrame` is traced; exceeding a
  budget (default 0.5 ms CPU) for K consecutive frames drops optional work, such as
  masks, before the kneeboard itself.
- R5. **Pass-through by default.** Any unknown or unsupported runtime state means
  we forward the game's call unmodified.

### 4.3 Plugin architecture (decided by owner)

- Open to outside developers. **Plugins never run in the game process.**
  Plugins that affect what the game shows submit *data* (shapes, poses,
  textures); only `okb-xr-layer` draws it.
- **Sandbox:** WebAssembly components under `wasmtime`. **Contract:** a WIT
  world `openkneeboard:plugin@1.x`, semver-versioned; minor versions only add
  to it.
- **Safeguards:** per-plugin memory cap; CPU time budget (fuel or epoch interruption);
  a hung plugin is killed and the user is notified; a crash is contained to that plugin.
- **Capability permissions** are declared in the manifest and approved by the user at install;
  undeclared capabilities are not linked into the sandbox at all. Initial
  capabilities: `vr.passthrough-mask.publish`,
  `ui.tab`, `storage.plugin-dir`, `net.http` (allow-listed hosts).
- **Manifest:** `v2.json` extends today's `v1.json` (same `ID` and `Metadata`
  rules) with `Components` (WASM files) and `Permissions`. `v1.json` web-tab plugins keep
  working unchanged.
- **Built-ins use the same contract:** the passthrough module ships as the first
  plugin, to prove the contract works.
- Signing and a plugin catalog: deferred; see Section 7.

### 4.4 Passthrough mask module (decided by owner unless marked)

Two **separate, out-of-game** steps, each its own process:

1. **Mask definition** (`okb-mask-editor`): the user draws a **2D mask**, a set of
   closed polygons in the mask's own flat 2D coordinates (meters). The shape knows
   nothing about the room. Authoring UI is the builder's call (desktop window,
   VR editor, or both). Stored as JSON in the passthrough plugin's storage folder.
2. **Alignment** (Section 4.5): finds real-world reference markers and produces a
   **reference origin**; it is defined and run independently of the shape points.

In game, the mask is placed exactly the way kneeboards are placed today, except
that the origin comes from the references instead of the headset:

- Today, Recenter (`src/lib/VRKneeboard.cpp:77`) builds
  `mRecenter = RotationY(hmd yaw) * Translation(hmd x, 0, z)` and every kneeboard
  pose is multiplied by it.
- Masks use `maskPose * referenceOrigin`, where `referenceOrigin` is also
  **gravity-aligned (yaw + translation only)**, so it is the same kind of transform as
  `mRecenter`. Masks follow normal headset tracking like kneeboards do.
- Alignment moves **masks only**; kneeboards keep today's Recenter unchanged
  (my call).

**Data flow (my call):** editor and aligner save JSON in the plugin's storage folder;
the passthrough plugin (WASM) loads it and publishes the mask polygons and the
reference origin via `vr.passthrough-mask.publish`; the app writes them to a
separate, versioned, lock-free SHM section for masks; `okb-xr-layer` draws them.
Phase 4 therefore depends on Phase 3.

**In-game rendering methods, chosen by runtime capability probe** (user override
Auto / A / B / C / Off):

- **Method C, chroma-key punch: primary for Quest + Virtual Desktop.** VDXR
  implements no passthrough extension and reports only `XR_ENVIRONMENT_BLEND_MODE_OPAQUE`
  (verified in the VDXR source, `system.cpp:255`). Virtual Desktop (≥ 1.29) instead
  replaces a configurable chroma-key color in the streamed image with passthrough video.
  The layer adds a quad layer on top of the game, shaped as the mask and filled with
  the exact key color, with no feathering (feathering would blend the key color).
  Requirements: the user enables passthrough with chroma keying in VD's Streaming tab
  and sets the same key color in OpenKneeboard (setting, default pure green
  `#00FF00`). Caveats: the key color must not appear elsewhere in the frame;
  video compression may soften the edges.
- **Method A, `XR_FB_passthrough` + `XR_FB_triangle_mesh`:** for runtimes that
  offer it (for example, Meta's Quest Link runtime). The mask polygons, triangulated, become a
  projected-surface passthrough layer inserted above the game's projection layer.
- **Method B, alpha blend:** for runtimes reporting `XR_ENVIRONMENT_BLEND_MODE_ALPHA_BLEND`
  (candidates: Varjo, Pimax; unverified). **My call:** the layer never writes into
  game-owned swapchain images. It copies the game's projection images into its own
  swapchain, rasterizes the mask into premultiplied alpha, sets
  `XR_COMPOSITION_LAYER_BLEND_TEXTURE_SOURCE_ALPHA_BIT`, and submits the copy in place of
  the game's layer. On failure or budget overrun, it submits the game's original layer (R5).
- Auto prefers A, then B, then C. Edge feather setting for A/B (default 2 cm).

**Hardware priority:** 1) Quest 3 via Virtual Desktop (Method C); lower priority:
2) Pimax, 3) Varjo (probe, then A or B).

### 4.5 Reference markers and alignment (decided by owner unless marked)

**Markers:** both kinds are supported and work the same way:
- **Printed target stickers** (fiducials, for example AprilTag/ArUco; the family is the builder's call).
- **Recognized room features:** image-matched from reference photos the user captures
  of distinctive surfaces (for example, a cockpit panel).

Each marker has a known pose relative to the mask's 2D frame, recorded once during
setup. From 1+ detected markers (2+ recommended), the aligner solves a **gravity-aligned
yaw + translation** fit (least squares over marker positions; reject fits with RMS error
above a default 1.5 cm) and outputs `referenceOrigin`.

**Where recognition runs: depends on camera access.**
- **Quest (Virtual Desktop):** no PC-side camera access exists (VDXR has none, and
  Meta's Passthrough Camera API is on-headset Android only). Alignment therefore runs
  in an **on-headset companion app** (`okb-align-quest`, Android, Passthrough Camera
  API, Horizon OS v74+) that the user runs **before** starting the VD session. It
  computes `referenceOrigin` in the Quest's stage (floor) space and sends it to the
  PC app over the LAN. During play, Quest's own inside-out tracking keeps stage
  space anchored to the room, so the mask stays aligned. The layer watches
  `XrEventDataReferenceSpaceChangePending` (user or VD recenter) and re-applies the
  stored origin relative to the new space.
- **Varjo (lower priority):** `XR_VARJO_marker_tracking` gives true in-session
  detection of printed markers; image matching via Varjo's camera API is the builder's
  call. **Re-scans happen during play at the user's re-scan frequency setting**
  (default every 5 s; range 1 s–5 min, or "on recenter only").
- **Pimax (lower priority):** camera access is unknown; the Phase 6 probe decides
  whether it follows the Varjo path or the "align before session" path.
- On Quest, the frequency setting does not apply in-session (shown as unavailable,
  with the reason).

**Safety:** if a re-scan fails or a fit is rejected, the mask keeps its last good
pose; it never jumps to an unvalidated pose. A drift above tolerance (default
1 cm / 0.5°) shows an in-headset notice.

## 5. Phases

Each phase ends green in CI. Per `port-to-rust` (rewrite-rs skills), **porting
and behavior changes are separate steps**: a port first matches the old behavior, and
hardening follows as its own step.

- **Phase 0: Baseline and harness.** Add a CTest + GoogleTest (or Catch2) target for
  C++ characterization tests (SHM protocol, `RayIntersectsRect`, geometry,
  semver, settings JSON round-trips, plugin manifest parsing, `VRKneeboard` recenter
  math), measured with OpenCppCoverage. Add a Rust workspace with `cargo-nextest` and
  `cargo-llvm-cov`. Add a CI job that publishes both coverage numbers. Write the parity
  contract for SHM and the OpenXR layer (layer poses, sizes, timing tolerances).
- **Phase 1: Port SHM + the OpenXR layer to Rust (matching behavior).** Run the old and new
  side by side against the same frames (`test-feeder`, `hello_xr`). Ship behind the
  C++/Rust layer switch.
- **Phase 2: Isolation hardening.** Implement R1–R5. Fault-injection tests:
  kill the app mid-frame, hang it while holding old locks, never signal the fence,
  stall SHM. Pass criteria: game frame time within budget; no hang; no crash.
- **Phase 3: Plugin host.** wasmtime host, WIT `plugin@1.0`, `v2.json` manifest,
  permissions UI, resource limits, sample third-party plugin, contract docs.
  Tests: a malicious or hung plugin is contained.
- **Phase 4: Passthrough mask (Quest + VD first).**
  - 4a: capability probe tool (extensions, blend modes); record VDXR results.
  - 4b: 2D mask editor and mask SHM section; passthrough plugin.
  - 4c: Method C in the layer. **Hardware test on Quest 3 + VDXR** with a manual
    test script (pass/fail: mask edges within 1 cm of target, no visible key color
    outside the mask, frame-time impact within R4).
  - 4d: Methods A and B with mock-runtime tests; hardware-tested in Phase 6
    (A optionally on Quest Link).
- **Phase 5: Alignment.** Yaw + translation solver (unit-tested with synthetic
  markers); marker setup flow; `okb-align-quest` companion app (sticker detection, then
  image matching); LAN handoff to the PC app; recenter-event handling in the layer.
  Hardware test on Quest 3: align before session, then the mask holds through a
  VD recenter and a 30-minute session.
- **Phase 6 (lower priority): Pimax and Varjo.** Probe; hardware-test Method A/B;
  Varjo in-session marker tracking with the re-scan frequency setting; decide the Pimax
  alignment path.
- **Phase 7 (later): further Rust migration,** ranked by perf/safety value:
  window-capture hook, C API DLL, D3D/Vulkan render helpers, app-side frame
  compositing. The UI and CEF stay C++.

## 6. Constraints

- Windows 10/11 x64; MSVC + CMake + vcpkg remain the primary build; Rust is stable
  toolchain, MSRV pinned in `rust-toolchain.toml`.
- OpenXR loader layer conventions (implicit layer JSON, `xrNegotiateLoaderApiLayerInterface`)
  stay compatible with the existing installer (`OpenKneeboard-Installer`).
- SHM wire format: versioned; the old and new readers and writers must refuse
  mismatched versions cleanly (no overlay, no crash).
- Settings and profile JSON stay backward compatible.
- `unsafe` Rust is confined to the FFI/D3D/Vulkan/OpenXR boundary modules,
  each block with a `// SAFETY:` justification.
- US English in docs and comments.

## 7. Left to the builder

- C++ test framework (GoogleTest vs Catch2), CMake↔Cargo integration (Corrosion
  vs custom), FFI tool (cbindgen vs cxx).
- Exact lock-free SHM design (seqlock vs atomic slot index), within R1.
- WIT interface details beyond the listed capabilities.
- Mask editor UX (desktop and/or VR), fiducial family, image-matching algorithm.
- Default values above may be tuned after hardware testing.
- Deferred: plugin signing and a catalog or store, native plugins, UI rewrite.

## 8. Risks

- Method C depends on Virtual Desktop's chroma-key feature, which is user-configured and
  not controlled by any API; VD may change it. The key color can collide with game
  content. The fallback is the Quest Link runtime with Method A.
- The Quest companion app needs camera permission and Horizon OS v74+; distributing
  it (sideload vs Horizon Store) is unresolved.
- Alignment on Quest relies on stage space staying consistent between the companion app
  and the VD session; the Phase 5 hardware test verifies this.
- Method B depends on games' alpha output and premultiplication; some
  games may write garbage alpha. Mitigation: the layer writes its own alpha
  mask instead of trusting the game's.
- Game anti-cheat may object to API layers (an existing risk, unchanged).
