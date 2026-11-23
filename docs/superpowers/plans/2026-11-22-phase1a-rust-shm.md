# Phase 1a: Rust SHM Transport (Matching Behavior) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port the v1 SHM frame transport (`src/lib/SHM.cpp`, `src/lib/SHM/ActiveConsumers.cpp`) to Rust in `okb-shm`. The port must be byte-for-byte compatible with the C++ code. A C++ differential harness proves parity through a C ABI (`okb-ffi`).

**Architecture:**
- **`okb-shm`:** gains three things:
  - `#[repr(C)]` mirrors of every shared structure, checked against a layout manifest that a C++ test also checks;
  - a Win32 boundary module, the only place `unsafe` is allowed;
  - a `Writer`, a `Reader`, and ActiveConsumers code with the same observable behavior as the C++.
- **`okb-ffi`:** a thin `extern "C"` wrapper (cbindgen header), built by CMake through Corrosion.
- **`SHMParity_test.cpp`:** runs the same writer scripts through the C++ and the Rust writers and readers and compares the results.
- **Product build:** unchanged. Nothing in the product links Rust in Phase 1a. Phase 1b (the OpenXR layer) is a separate plan and consumes `okb-shm::Reader`.

**Tech Stack:** Rust 1.99.1 (`rust/rust-toolchain.toml`), `windows-sys` (Win32), cbindgen, Corrosion via CMake FetchContent, GoogleTest (Phase 0 harness), CMake 3.31 + MSVC v143.

**Spec:** `docs/superpowers/specs/2026-11-22-rust-port-plugins-passthrough-design.md` (Section 4.1 "SHM migration", Section 5 Phase 1, Section 6). **Definition of done:** `docs/internals/rust-port-parity-contract.md`.

## Global Constraints

- Phase 1 "keeps the existing mutex-based wire format byte-for-byte" (spec 4.1): **no behavior changes**, including known quirks. Lock-free reads, bounded waits, and no-crash rules are Phase 2.
- `unsafe` Rust is confined to the FFI/D3D/Vulkan/OpenXR boundary modules, each block with a `// SAFETY:` justification (spec 6). Here that means `okb-shm/src/win32.rs` and `okb-ffi` only.
- The Rust layer builds for both `x86_64` and `i686` (spec 4.1). The wire layout must be identical on both.
- Default product build unchanged; Rust is built by CMake only with `-DOPENKNEEBOARD_BUILD_TESTS=ON` in this phase.
- Static CRT, to match OpenKneeboard's hybrid CRT (`CMakeLists.txt`): Rust MSVC targets build with `-C target-feature=+crt-static`.
- Rust code builds clean with `cargo clippy --all-targets` (workspace `clippy::all = deny`) and `cargo fmt --check`.
- Dependencies: only `windows-sys` (runtime) and `serde_json` (dev-dependency, manifest test). No other crates without a ledger ruling.
- US English in docs and comments.

## Review Focus

1. **An app crash while holding the SHM mutex** (abandoned mutex): the next `lock()` must succeed and reset the whole header to defaults, as C++ does on `WAIT_ABANDONED`. Pinned by `abandoned_mutex_resets_header` (Task 3).
2. **A 32-bit reader against the 64-bit writer:** every wire struct must have the same size and offsets on `i686`, and handles stay 64-bit wide. Pinned by `const` size assertions plus `cargo build --target i686-pc-windows-msvc` in CI (Tasks 2 and 6).
3. **A writer replaced while a reader holds the previous session's handles:** the reader must call its session-changed callback once and drop the stale handles. Pinned by the `DetachAndReattach` parity script (Task 7).
4. **The feeder process is gone** (`OpenProcess` fails for `mFeederProcessID`): C++ returns before duplicating handles, so the reader gets `UnusableHandles`, not a crash. Pinned by `dead_feeder_pid_gives_unusable_handles` (Task 5).
5. **More than `MAX_VIEW_COUNT` layers submitted:** C++ calls `fatal()`, so Rust must also terminate (panic in the app process). Pinned by `submit_frame_panics_over_max_view_count` (Task 4).

---

### Task 1: Move wire structs into a header and pin a layout manifest

The shared structs live in an anonymous namespace inside `SHM.cpp`, so tests can't see them. This task moves them unchanged and pins their layout.

**Files:**
- Create: `src/lib/include/OpenKneeboard/SHM/Wire.hpp`: move `HeaderFlags`, `FixedSizeHandle`, `SharedData` (with nested `Frame`), and the `SHM_SIZE` `static_assert` out of `src/lib/SHM.cpp` into `namespace OpenKneeboard::SHM::Wire`. Keep the code byte-identical. `is_bitflags_v<HeaderFlags>` moves with it.
- Modify: `src/lib/SHM.cpp`: include `Wire.hpp`, `using namespace Wire;`, and delete the moved code.
- Create: `rust/crates/okb-shm/layout/shm-v1.json`: the manifest.
- Create: `src/tests/SHMLayout_test.cpp`; Modify: `src/tests/CMakeLists.txt` (add the file; add `OKB_RUST_DIR="${CMAKE_SOURCE_DIR}/rust"` to the compile definitions).

**Interfaces:**
- Produces: manifest schema `{"structs": {"<Name>": {"size": N, "align": N, "fields": {"<field>": offset}}}}` for `SharedData`, `SharedData::Frame` (key `Frame`), `Config`, `LayerConfig`, `VRRenderSettings`, `VRRenderSettings::Quirks` (key `Quirks`), `VRLayer`, `VRPose`, `GazeTargetScale`, `VROpacitySettings`, `PixelRect`, `PixelSize`, `ActiveConsumers`. Field keys are the C++ member names (`mGPULUID`, ...). Task 2 reads this file.

- [ ] **Step 1: Write the test** `SHMLayout.MatchesManifest`. It builds the actual layout with `sizeof`, `alignof` and `offsetof` for every listed field, as an `nlohmann::json`. It loads `OKB_RUST_DIR "/crates/okb-shm/layout/shm-v1.json"` and `EXPECT_EQ`s the two, printing `actual.dump(2)` on failure. If the env var `OKB_UPDATE_SHM_LAYOUT=1` is set, it writes `actual` to the file and calls `GTEST_SKIP()`.
- [ ] **Step 2: Run it, expect FAIL** (no manifest yet): `ctest --test-dir build-tests -C Debug -R SHMLayout --output-on-failure`.
- [ ] **Step 3: Generate the manifest.** Run with `OKB_UPDATE_SHM_LAYOUT=1`, then rerun without it. Expected: PASS. Check that `SharedData.size == 3024` in the file.
- [ ] **Step 4: Run the full C++ suite**: 27 tests, 0 failures (the move changed no behavior).
- [ ] **Step 5: Commit**: `refactor(shm): move wire structs to SHM/Wire.hpp and pin a v1 layout manifest`

---

### Task 2: Rust wire types checked against the manifest

**Files:**
- Modify: `rust/Cargo.toml`:
  - `[workspace.lints.rust] unsafe_code = "deny"` (was `forbid`, which can't be relaxed per module);
  - `[workspace.lints.clippy] undocumented_unsafe_blocks = "deny"`.
- Modify: `rust/rust-toolchain.toml`: `targets = ["x86_64-pc-windows-msvc", "i686-pc-windows-msvc"]`.
- Create: `rust/.cargo/config.toml`: `[target.'cfg(all(windows, target_env = "msvc"))'] rustflags = ["-C", "target-feature=+crt-static"]`.
- Create: `rust/crates/okb-shm/src/wire.rs`; Modify: `src/lib.rs` (`pub mod wire;`).
- Create: `rust/crates/okb-shm/tests/layout.rs`; Modify: `okb-shm/Cargo.toml` (`[dev-dependencies] serde_json`).

**Interfaces:**
- Consumes: `layout/shm-v1.json` (Task 1).
- Produces (all `#[repr(C)]`, `Clone, Copy`, with `Default` matching the C++ default member initializers):
  - `SharedData`, `Frame`, `Config`, `LayerConfig`, `VrRenderSettings`, `Quirks`, `VrLayer`, `VrPose`, `GazeTargetScale`, `VrOpacitySettings`, `PixelRect`, `PixelSize`, `ActiveConsumers`;
  - `FixedSizeHandle(u64)`;
  - `HeaderFlags` (`const FEEDER_ATTACHED: u32 = 1`);
  - `pub const MAGIC: u64 = u64::from_le_bytes(*b"OKBMagic");`;
  - `pub const SHM_SIZE: usize = 3024;`;
  - `SharedData::have_feeder(&self) -> bool`;
  - `SharedData::new(session_id: u64) -> SharedData`: magic set, everything else default.
- C++ `bool` fields are Rust `bool`. C++ enums are `i32` newtypes (`Quirks::Upscaling`, `PixelRect` origin). `ActiveConsumers` time points are `i64` nanoseconds.

- [ ] **Step 1: Write the failing test** `layout_matches_manifest`. For every struct and field in the manifest, assert that `size_of`, `align_of` and `core::mem::offset_of!` equal the manifest values, with messages like `"SharedData.mGPULUID offset"`. Keep a `match` from C++ field name to Rust `offset_of!`, and fail on any manifest field that has no Rust mapping.
- [ ] **Step 2: Run it, expect FAIL**: `cd rust; cargo nextest run -p okb-shm` (types missing).
- [ ] **Step 3: Implement `wire.rs`**, plus `const _: () = assert!(size_of::<SharedData>() == SHM_SIZE);`. The `const` assertion is what proves the `i686` layout.
- [ ] **Step 4: Verify on both targets**: `cargo nextest run -p okb-shm` passes; `cargo build -p okb-shm --target i686-pc-windows-msvc` succeeds; clippy and fmt are clean.
- [ ] **Step 5: Commit**: `feat(okb-shm): repr(C) v1 wire types checked against the C++ layout manifest`

---

### Task 3: Win32 boundary (names, mapping, named mutex)

**Files:**
- Create: `rust/crates/okb-shm/src/win32.rs` (`#![allow(unsafe_code)]` at module top; the only unsafe module in `okb-shm`) and `src/names.rs`; Modify: `okb-shm/Cargo.toml` (`windows-sys` with the features this module needs: `Win32_Foundation`, `Win32_System_Memory`, `Win32_System_Threading`, `Win32_Security`, `Win32_Security_Cryptography`).

**Interfaces:**
- Produces:
  - `pub struct Version { pub major: u32, pub minor: u32, pub patch: u32, pub build: u32 }`
  - `pub struct Names { pub frames: String, pub mutex: String, pub active_consumers: String }` and `Names::new(reverse_domain: &str, v: Version) -> Names`
  - `pub(crate) struct Mapping` with `Mapping::open(name: &str, size: usize) -> io::Result<(Mapping, bool /*created*/)>`, `as_ptr(&self) -> *mut u8`, and `Drop` (unmap and close)
  - `pub(crate) struct NamedMutex` with `open(name: &str) -> io::Result<NamedMutex>`, `lock(&self) -> io::Result<LockOutcome>`, `try_lock(&self) -> io::Result<Option<LockOutcome>>`, `unlock(&self)`
  - `pub(crate) enum LockOutcome { Acquired, Abandoned }`
  - `pub(crate) fn session_id() -> u64`: `(GetCurrentProcessId() as u64) << 32 | random u32`, with the random value from `BCryptGenRandom`
  - `pub(crate) fn is_elevated() -> bool`
  - `pub(crate) fn steady_clock_now_ns() -> i64`: matches MSVC `std::chrono::steady_clock::now()`, `ns = (qpc / freq) * 1_000_000_000 + (qpc % freq) * 1_000_000_000 / freq`

- [ ] **Step 1: Write the failing tests** (in `names.rs` and `tests/win32.rs`; each test uses its own reverse domain `"okb-test.<test>.<pid>"` so tests never touch the app's SHM):
  - `names_match_cpp_format`: `Names::new("com.openkneeboard", Version{major:1,minor:13,patch:0,build:0})` gives:
    - `frames == "com.openkneeboard/1.13.0.0-sbd0"`
    - `mutex == "com.openkneeboard/1.13.0.0-sbd0.mutex"`
    - `active_consumers == format!("com.openkneeboard/1.13.0.0/ActiveConsumers-s{:x}", size_of::<ActiveConsumers>())`
  - `mapping_is_shared_between_opens`: two `Mapping::open`s of one name; a byte written through one is read through the other; the first reports `created == true`, the second `false`.
  - `abandoned_mutex_resets_header` [Review Focus 1]: a spawned thread `lock()`s and exits without `unlock()`; the next `lock()` returns `Abandoned`. (The header reset itself is asserted in Task 4, where `Writer` owns the reset.)
  - `steady_clock_matches_cpp_formula`: two calls are non-decreasing, and the value is within 1 s of the formula applied to a direct QPC read.
- [ ] **Step 2: Run them, expect FAIL** (missing items).
- [ ] **Step 3: Implement.** Every `unsafe` block carries a `// SAFETY:` comment. Wide strings for `CreateFileMappingW`/`CreateMutexW` come from `encode_utf16().chain([0])`.
- [ ] **Step 4: Run them, expect PASS**; clippy and fmt clean.
- [ ] **Step 5: Commit**: `feat(okb-shm): Win32 boundary for named mapping, mutex, session id and clock`

---

### Task 4: Rust `Writer`

**Files:**
- Create: `rust/crates/okb-shm/src/writer.rs`; Test: `rust/crates/okb-shm/tests/writer.rs`.

**Interfaces:**
- Consumes: `wire::*`, `win32::{Mapping, NamedMutex, LockOutcome, session_id}`, `Names` (Tasks 2 and 3).
- Produces:
  - `Writer::new(names: &Names, gpu_luid: u64) -> Option<Writer>`: `None` where C++ `Writer::operator bool` is false. On success it overwrites the header with `SharedData::new(session_id())`, without taking the lock (quirk: same as C++).
  - `Writer::lock(&mut self) -> WriterGuard<'_>` and `Writer::try_lock(&mut self) -> Option<WriterGuard<'_>>`. On `Abandoned`, the header is reset to `SharedData::new(session_id())` (C++ `*mHeader = {}`, which also creates a new session ID). The guard unlocks on drop.
  - `WriterGuard::submit_empty_frame(&mut self)`, `begin_frame(&mut self) -> NextFrameInfo { texture_index: u8, fence_out: i64 }`, `submit_frame(&mut self, info: NextFrameInfo, config: &Config, layers: &[LayerConfig], texture: u64, fence: u64)`, `detach(&mut self)`, `frame_count(&self) -> u64`
  - `Drop for Writer`: lock, then detach (as the C++ destructor does).
  - The C++ `WriterState` machine is replaced by types. The guard is the `Locked` state, and calling `submit_frame` without a matching `begin_frame` is a `debug_assert!`.

- [ ] **Step 1: Write the failing tests:**
  - `empty_frame_does_not_attach_feeder`: after `submit_empty_frame`, the header has `have_feeder() == false` and `mFrameNumber == 1`.
  - `submit_frame_sets_feeder_fields`: `mGPULUID == luid`, the `FEEDER_ATTACHED` flag is set, `mFeederProcessID == current pid`, the slot `frame_number % 2` holds the config and layers, `mLayerCount == layers.len()`, and `mReadyReadFenceValue == info.fence_out`.
  - `begin_frame_index_formula`: `texture_index == (frame_number + 1) % 2`, and `fence_out` counts 1, 2, 3...
  - `detach_creates_new_session_and_clears_feeder`
  - `abandoned_lock_resets_header`: a thread takes the raw mutex and exits; `lock()` then shows a fresh header with a new session ID.
  - `submit_frame_panics_over_max_view_count` (`#[should_panic]`, 17 layers) [Review Focus 5]
- [ ] **Step 2: Run them, expect FAIL.**
- [ ] **Step 3: Implement `writer.rs`.** Mapped memory is read and written through `win32`'s pointer accessors only (`read_header() -> SharedData` / `write_header(&SharedData)` and per-field helpers), so `writer.rs` itself has no `unsafe`.
- [ ] **Step 4: Run them, expect PASS**; clippy and fmt clean.
- [ ] **Step 5: Commit**: `feat(okb-shm): Rust SHM writer matching C++ v1 behavior`

---

### Task 5: Rust `Reader` and ActiveConsumers

**Files:**
- Create: `rust/crates/okb-shm/src/reader.rs`, `src/active_consumers.rs`; Test: `tests/reader.rs`.

**Interfaces:**
- Consumes: Tasks 2–4.
- Produces:
  - `#[repr(u32)] pub enum ConsumerKind { OpenVr = 1, OpenXrD3d11 = 2, OpenXrD3d12 = 4, OpenXrVulkan2 = 8, Viewer = u32::MAX }`
  - `pub enum FrameError { IncorrectGpu, NoFeeder, NoLayers, UnusableHandles }`: same variants as C++ `Frame::Error`.
  - `pub enum ReadError { Frame(FrameError), DuplicateHandle(io::Error) }`: C++ *throws* when `DuplicateHandle` fails; Rust returns this variant, and the FFI shim (Task 6) rethrows it.
  - `pub struct Frame { pub config: Config, pub layers: Vec<LayerConfig>, pub texture: u64, pub fence: u64, pub fence_in: i64, pub index: u8 }`
  - `Reader::new(names: &Names, kind: ConsumerKind, gpu_luid: u64, on_session_changed: F) -> Option<Reader<F>>` where `F: FnMut()`
  - `Reader::has_feeder(&self) -> bool`
  - `Reader::maybe_get(&mut self) -> Result<Frame, ReadError>`
  - `Reader::frame_count(&self) -> u64`
  - `active_consumers::set(names: &Names, kind: ConsumerKind)`, which preserves the quirk that `OpenXrD3d12` writes `mOpenXR_D3D11`. Also `set_active_in_game_view_id(names: &Names, id: u64)` and `get(names: &Names) -> ActiveConsumers`. These open the ActiveConsumers mapping once per process per name, and zero it only when they created it (as the C++ code does).
- `maybe_get` order, verbatim from C++:
  1. no feeder → `NoFeeder` (checked before and after locking);
  2. update the session, calling `on_session_changed` when it changed;
  3. GPU mismatch → `IncorrectGpu`;
  4. `active_consumers::set`;
  5. zero layers → `NoLayers`;
  6. missing handles → `UnusableHandles`.

  The session update keeps the C++ `UpdateSession` semantics exactly: open the feeder process with `PROCESS_DUP_HANDLE` once per session, skip duplication when `mLayerCount == 0`, and re-duplicate only when the foreign handle value changes.

- [ ] **Step 1: Write the failing tests** (each with its own test domain):
  - `error_precedence_matches_cpp`:
    - no writer → `NoFeeder`;
    - empty frame → `NoFeeder`;
    - zero-layer frame, wrong LUID → `IncorrectGpu`;
    - the same with the right LUID → `NoLayers`.
  - `usable_handles_round_trip`: the writer submits a frame whose texture and fence are two `CreateEventW` handles; the reader returns `Ok(frame)` with non-zero `texture`/`fence` that refer to the same kernel objects (`CompareObjectHandles`), plus matching config, layers, fence, and `index == frame_number % 2`.
  - `session_callback_fires_once_per_session`: two reads in one session → 1 call; after detach and resubmit → 2 calls.
  - `dead_feeder_pid_gives_unusable_handles` [Review Focus 4]: a hand-written header with the feeder flag, 1 layer, the right LUID and `mFeederProcessID = 0` → `UnusableHandles`.
  - `d3d12_consumer_sets_d3d11_timestamp_quirk`
- [ ] **Step 2: Run them, expect FAIL.**
- [ ] **Step 3: Implement.** Duplicated handles are owned with close-on-drop, in a `win32::OwnedHandle`.
- [ ] **Step 4: Run them, expect PASS**; clippy and fmt clean.
- [ ] **Step 5: Commit**: `feat(okb-shm): Rust SHM reader and ActiveConsumers matching C++ v1 behavior`

---

### Task 6: `okb-ffi` C ABI, Corrosion build, and CI

**Files:**
- Create: `rust/crates/okb-ffi/{Cargo.toml,src/lib.rs,cbindgen.toml}` (`crate-type = ["staticlib"]`; `#![allow(unsafe_code)]`) and `rust/crates/okb-ffi/include/okb_ffi.h` (generated, checked in).
- Create: `third-party/corrosion.cmake`: `FetchContent_Declare(Corrosion URL https://github.com/corrosion-rs/corrosion/archive/refs/tags/<latest v0.x tag at task start>.zip URL_HASH SHA256=<computed>)`, matching `third-party/bindline.cmake`. Include it only when `OPENKNEEBOARD_BUILD_TESTS` is on, and call `corrosion_import_crate(MANIFEST_PATH rust/Cargo.toml CRATES okb-ffi)`.
- Modify: `src/tests/CMakeLists.txt` (link `okb-ffi`, add its include dir); `.github/workflows/ci.yml`:
  - Build (Debug): add a `rustup toolchain install` step (in `source/rust`) before Configure;
  - Rust job: add a step `cargo build --workspace --target i686-pc-windows-msvc`, plus a step that runs cbindgen and `git diff --exit-code crates/okb-ffi/include`.

**Interfaces:**
- Consumes: `okb_shm::{Names, Version, Writer, Reader, ConsumerKind, Frame, FrameError, ReadError, wire::*}`.
- Produces the C ABI (opaque handles; `wire` structs are passed by pointer with their `repr(C)` layout, so C++ casts `SHM::Config*`/`SHM::LayerConfig*` directly):
  - `OkbShmNames* okb_shm_names_new(const char* reverse_domain, uint32_t major, uint32_t minor, uint32_t patch, uint32_t build)` / `okb_shm_names_free`
  - `OkbShmWriter* okb_shm_writer_new(const OkbShmNames*, uint64_t gpu_luid)` (null when invalid) / `okb_shm_writer_free`
  - `void okb_shm_writer_lock` / `bool okb_shm_writer_try_lock` / `void okb_shm_writer_unlock` (the FFI keeps the guard internally, matching the C++ Lockable interface)
  - `okb_shm_writer_submit_empty_frame`, `OkbNextFrameInfo okb_shm_writer_begin_frame`, `okb_shm_writer_submit_frame(OkbShmWriter*, OkbNextFrameInfo, const OkbConfig*, const OkbLayerConfig*, size_t count, uint64_t texture, uint64_t fence)`, `okb_shm_writer_detach`, `uint64_t okb_shm_writer_frame_count`
  - `OkbShmReader* okb_shm_reader_new(const OkbShmNames*, uint32_t kind, uint64_t gpu_luid, void (*on_session_changed)(void*), void* ctx)` / `okb_shm_reader_free`, `bool okb_shm_reader_has_feeder`
  - `int32_t okb_shm_reader_maybe_get(OkbShmReader*, OkbFrame* out)`: 0 on success; otherwise `1 + FrameError` in C++ enum order (`IncorrectGPU`=1, `NoFeeder`=2, `NoLayers`=3, `UnusableHandles`=4), or `-(win32 error)` for `ReadError::DuplicateHandle`. `OkbFrame` is `{OkbConfig config; OkbLayerConfig layers[16]; uint32_t layer_count; uint64_t texture; uint64_t fence; int64_t fence_in; uint8_t index;}`.
  - Every entry point catches panics with `std::panic::catch_unwind` and aborts the process on one, so no unwinding crosses the C ABI (this preserves "terminate" from Phase 1's point of view).

- [ ] **Step 1: Write the failing C++ smoke test** `SHMFfi.WriterReaderRoundTrip` (in `src/tests/SHMParity_test.cpp`, with a test-only reverse domain): Rust writer → zero-layer frame → Rust reader `maybe_get` returns `3` (`NoLayers`).
- [ ] **Step 2: Configure and build, expect FAIL** (`okb-ffi` target missing): `cmake -B build-tests --preset vcpkg -G "Visual Studio 18 2026" -T v143 -DOPENKNEEBOARD_BUILD_TESTS=ON; cmake --build build-tests --config Debug --target OpenKneeboard-Tests`.
- [ ] **Step 3: Implement `okb-ffi` and the CMake/Corrosion wiring;** generate the header with `cbindgen --config cbindgen.toml --output include/okb_ffi.h`.
- [ ] **Step 4: Build and run**: `ctest --test-dir build-tests -C Debug -R SHMFfi --output-on-failure` passes, and the full suite stays green. `cmake -B build-default --preset vcpkg ...` (no test option) does not fetch Corrosion; check that `build-default/_deps` has no `corrosion-src`.
- [ ] **Step 5: Commit**: `feat(okb-ffi): C ABI for okb-shm, built via Corrosion for tests; CI builds i686`

---

### Task 7: Differential parity harness

**Files:**
- Modify: `src/tests/SHMParity_test.cpp`.
- Modify: `docs/internals/rust-port-parity-contract.md`:
  - add the `ActiveConsumers::Set(OpenXR_D3D12)` → `mOpenXR_D3D11` quirk to "Known quirks", citing `d3d12_consumer_sets_d3d11_timestamp_quirk`;
  - under "How parity is measured", state that byte comparison masks `mSessionID` (random per writer and per detach).

**Interfaces:**
- Consumes: the C ABI (Task 6); C++ `SHM::Writer`/`SHM::Reader`.
- Produces: `struct Step` (variant of `Lock`, `Unlock`, `SubmitEmptyFrame`, `SubmitFrame{config, layers, useHandles}`, `Detach`, `Read{readerIndex}`), and `struct Trace { std::vector<ReadResult> reads; std::vector<std::array<std::byte, 3024>> snapshots; std::vector<int> sessionCallbacks; }`. `RunScript(Impl writer, Impl reader, std::span<const Step>) -> Trace` runs one writer/reader pairing under the real app names (the C++ side is hard-wired to them; the fixture skips if OpenKneeboard is running, as `SHMTest` does). `ExpectSameTrace(a, b)` compares reads field by field, compares snapshots with the 8 bytes of `mSessionID` masked (offset `offsetof(Wire::SharedData, mSessionID)` from `Wire.hpp`, not a hard-coded number), and compares callback counts. Handle values are compared as "usable or not" via `CompareObjectHandles` against the event handles the script created.

- [ ] **Step 1: Write the parity tests.** Each script runs in all four pairings (C++/Rust writer × C++/Rust reader), and every Rust-involving trace must equal the C++/C++ trace:
  - `SHMParity.EmptyAndZeroLayerFrames`
  - `SHMParity.MultiLayerFrame` (3 layers with distinct poses, sizes, opacity and `mLocationOnTexture`, and real event handles)
  - `SHMParity.SwapchainWrapAround` (5 submits, with a read after each)
  - `SHMParity.WrongGpu`
  - `SHMParity.DetachAndReattach` [Review Focus 3]
  - `SHMParity.ActiveConsumers` (after a passing read, the same `ActiveConsumers` field became non-zero for each consumer kind, including the D3D12 quirk)
- [ ] **Step 2: Run them**: `ctest --test-dir build-tests -C Debug -R SHMParity --output-on-failure`. Expected: all pass. Any difference is a Rust bug: fix it in Tasks 4–5 code under superpowers:systematic-debugging, never by loosening the comparison.
- [ ] **Step 3: Check that the harness can fail.** Temporarily make the Rust `Writer` skip `FEEDER_ATTACHED` in `submit_frame`; `SHMParity.MultiLayerFrame` must fail naming the differing read; then revert.
- [ ] **Step 4: Run the full C++ and Rust suites and coverage** (`scripts/coverage-cpp.ps1`, `scripts/coverage-rust.ps1`). Expected: everything green; Rust line coverage is now non-zero. Record the number in the "Baseline" table of `docs/internals/testing.md` as a new row, "Rust line coverage after Phase 1a".
- [ ] **Step 5: Commit**: `test: C++/Rust SHM differential parity harness; record Phase 1a coverage`
