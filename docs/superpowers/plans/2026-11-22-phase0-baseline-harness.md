# Phase 0: Baseline and Test Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give OpenKneeboard its first automated tests (C++ characterization tests plus a Rust workspace), measure coverage for both, run them in CI, and write the parity contract that Phase 1's Rust port must meet.

**Architecture:** A new GoogleTest executable `OpenKneeboard-Tests` (built when `OPENKNEEBOARD_BUILD_TESTS=ON`) links existing `src/lib` static libraries and pins their current behavior. A Cargo workspace at `rust/` starts with one crate, `okb-shm`, which holds the SHM wire constants and a test that checks them against the C++ source. CI runs both suites with coverage (OpenCppCoverage for C++, `cargo-llvm-cov` for Rust) and publishes the numbers.

**Tech Stack:** CMake 3.31 + MSVC (VS 2022) + vcpkg (manifest mode), GoogleTest, CTest, OpenCppCoverage, Rust stable (`cargo-nextest`, `cargo-llvm-cov`), GitHub Actions `windows-2022`.

**Spec:** `docs/superpowers/specs/2026-11-22-rust-port-plugins-passthrough-design.md` (Phase 0, Section 5; coverage baseline, Section 2.1; isolation rules, Section 4.2)

## Global Constraints

- Windows 10/11 x64; MSVC + CMake + vcpkg remain the primary build.
- Rust: stable toolchain; MSRV pinned in `rust/rust-toolchain.toml` (pin the exact version `rustc --version` reports when Task 6 starts; today 1.99.1).
- This phase changes **no product behavior**: tests characterize current behavior. If a test exposes a bug, keep the test asserting current behavior and log the bug in the parity contract's "Known quirks" list.
- `unsafe` Rust: none in this phase.
- US English in docs and comments.
- Default product build must not change: tests build only with `-DOPENKNEEBOARD_BUILD_TESTS=ON`.

## Prerequisites (local machine)

- `git submodule update --init third-party/vcpkg` (the worktree has an empty `third-party/vcpkg`).
- Run CMake from a **VS 2022 Developer PowerShell** (`cmake` is not on the plain PATH).
- `choco install opencppcoverage` or the MSI from github.com/OpenCppCoverage/OpenCppCoverage; `cargo install cargo-nextest cargo-llvm-cov --locked`; `rustup component add llvm-tools-preview`.
- Close OpenKneeboard before running tests: SHM tests use the same named shared memory as the app.

## Review Focus

1. **A running OpenKneeboard instance during tests:** SHM tests would attach to the real app's shared memory and give false results. Expected: the SHM test fixture detects an existing feeder and calls `GTEST_SKIP()` with a clear message (Task 4).
2. **The default build gains a test dependency:** someone building without `-DOPENKNEEBOARD_BUILD_TESTS=ON` must not need GoogleTest. Expected: a `tests` vcpkg feature, requested only when the option is on (Task 1).
3. **Constant drift between C++ and Rust:** someone changes `SHMSwapchainLength` or `MaxViewCount` in C++ only. Expected: the Rust test fails, naming the constant (Task 6).
4. **A coverage step that silently measures nothing:** OpenCppCoverage without `--cover_children` reports 0% because CTest spawns the test process. Expected: CI fails if the Cobertura report shows zero covered lines (Task 7).
5. **The yaw-only recenter rule:** a head tilted or rolled at recenter time must not tilt kneeboards. Expected: pitch and roll at recenter time are ignored (Task 3).

---

### Task 1: Test target scaffold with the first test (semver)

**Files:**
- Modify: `CMakeLists.txt` (add option `OPENKNEEBOARD_BUILD_TESTS`, default `OFF`; when `ON`, add `tests` to `VCPKG_MANIFEST_FEATURES` **before** `project()`, and call `enable_testing()` + `add_subdirectory(src/tests)` after the existing subdirectories)
- Modify: `vcpkg.json` (add feature `"tests": { "description": "Build automated tests", "dependencies": ["gtest"] }`)
- Create: `src/tests/CMakeLists.txt`
- Create: `src/tests/semver_test.cpp`

**Interfaces:**
- Produces: CMake target `OpenKneeboard-Tests` (single GoogleTest executable, `gtest_main`, tests registered with `gtest_discover_tests`). Later tasks add `.cpp` files to it and link more libraries.
- Consumes: `OpenKneeboard-semver` (`CompareSemVer`, `CompareVersions`, `ToSemVerString` in `src/lib/include/OpenKneeboard/semver.hpp`).

- [ ] **Step 1: Write the failing test** in `src/tests/semver_test.cpp`

```cpp
TEST(SemVer, CompareOrdersByPrecedence) {
  EXPECT_EQ(CompareSemVer("1.2.3", "1.2.4"), ThreeWayCompareResult::LessThan);
  EXPECT_EQ(CompareSemVer("1.10.0", "1.9.0"), ThreeWayCompareResult::GreaterThan);
  EXPECT_EQ(CompareSemVer("1.2.3", "1.2.3"), ThreeWayCompareResult::Equal);
  EXPECT_EQ(CompareSemVer("1.2.3-beta", "1.2.3"), ThreeWayCompareResult::LessThan);
}
TEST(SemVer, CompareVersionsAcceptsShortForms) {
  // The plugin docs say OKB versions may have 1-4 components.
  EXPECT_EQ(CompareVersions("1.9", "1.9.0"), ThreeWayCompareResult::Equal);
  EXPECT_EQ(CompareVersions("1.13.0", "1.9"), ThreeWayCompareResult::GreaterThan);
}
TEST(SemVer, ToSemVerStringCharacterization) {
  // Characterization: record what the current code returns.
  EXPECT_EQ(ToSemVerString("1.2.3.4"), /* observed */ "...");
}
```
Replace `"..."` with the observed value after the first run, and write one line on the observed behavior in the test comment.

- [ ] **Step 2: Configure and build; verify it fails before the target exists, then compiles**

Run (Developer PowerShell, repo root):
`cmake -B build-tests --preset vcpkg -DOPENKNEEBOARD_BUILD_TESTS=ON; cmake --build build-tests --config Debug --target OpenKneeboard-Tests`
Expected: builds; `ctest --test-dir build-tests -C Debug -R SemVer` runs 3 tests (the characterization one fails until its value is filled in).

- [ ] **Step 3: Fill in the characterization value; rerun**

Run: `ctest --test-dir build-tests -C Debug -R SemVer --output-on-failure`
Expected: `100% tests passed, 0 tests failed out of 3`

- [ ] **Step 4: Verify the default build is untouched**

Run: `cmake -B build-default --preset vcpkg` (no test option)
Expected: configure succeeds and vcpkg does **not** install `gtest`.

- [ ] **Step 5: Commit**

```bash
git add CMakeLists.txt vcpkg.json src/tests
git commit -m "test: add GoogleTest harness behind OPENKNEEBOARD_BUILD_TESTS with semver tests"
```

---

### Task 2: RayIntersectsRect characterization

**Files:**
- Create: `src/tests/RayIntersectsRect_test.cpp`
- Modify: `src/tests/CMakeLists.txt` (link `OpenKneeboard-RayIntersectsRect`)

**Interfaces:**
- Consumes: `bool RayIntersectsRect(Vector3 rayOrigin, Quaternion rayOrientation, Vector3 rectCenter, Quaternion rectOrientation, Vector2 rectSize)` (`src/lib/include/OpenKneeboard/RayIntersectsRect.hpp`). OpenXR convention: a ray with identity orientation points down −Z.

- [ ] **Step 1: Write the tests**

```cpp
// Rect 1 m x 1 m at (0, 0, -2), facing the viewer (identity orientation).
TEST(RayIntersectsRect, HitsCenterStraightAhead)      // origin (0,0,0), identity  -> true
TEST(RayIntersectsRect, MissesWhenPointingAway)       // origin (0,0,0), yaw 180°  -> false
TEST(RayIntersectsRect, HitsInsideEdge)               // origin (0.49,0,0), identity -> true
TEST(RayIntersectsRect, MissesOutsideEdge)            // origin (0.51,0,0), identity -> false
TEST(RayIntersectsRect, MissesWhenRectBehindOrigin)   // rect at (0,0,+2)          -> false
TEST(RayIntersectsRect, RespectsRectRotation)         // rect yawed 90°: edge-on   -> false
```
Each test is one `EXPECT_TRUE`/`EXPECT_FALSE` with the values in its comment. If an observed result differs (for example, the behind-origin case), assert the observed value and add it to the parity contract's "Known quirks" (Task 8).

- [ ] **Step 2: Build and run**

Run: `cmake --build build-tests --config Debug --target OpenKneeboard-Tests; ctest --test-dir build-tests -C Debug -R RayIntersectsRect --output-on-failure`
Expected: `0 tests failed out of 6`

- [ ] **Step 3: Commit**

```bash
git add src/tests
git commit -m "test: characterize RayIntersectsRect"
```

---

### Task 3: VRKneeboard recenter and pose characterization

**Files:**
- Create: `src/tests/VRKneeboard_test.cpp`
- Modify: `src/tests/CMakeLists.txt` (link `OpenKneeboard-VRKneeboard`)

**Interfaces:**
- Consumes: `VRKneeboard::GetLayers(const SHM::Config&, const std::span<const SHM::LayerConfig>&, const Pose& hmdPose)` (protected); `VRRenderSettings::mRecenterCount`; `Layer::mRenderParameters.mKneeboardPose`.
- Produces: test helper `class TestableVRKneeboard : public VRKneeboard { public: using VRKneeboard::GetLayers; };` inside the test file only. Phase 5's "move kneeboards with masks" tests extend this file.

- [ ] **Step 1: Write the property tests** (one layer with `mVREnabled = true` and a fixed `mVR.mPose`)

```cpp
TEST(VRKneeboard, PoseIsStableWithoutRecenter)
  // Same hmd pose, two calls, same mRecenterCount -> identical kneeboard pose.
TEST(VRKneeboard, RecenterFollowsHeadsetTranslationInXZ)
  // Recenter at hmd (0,1.7,0); then bump mRecenterCount and recenter at (1,1.7,2).
  // Kneeboard position delta == (1, 0, 2) (within 1e-5).
TEST(VRKneeboard, RecenterUsesYawOnly)
  // Recenter with hmd yaw 90° vs yaw 90° + pitch 30° + roll 20°:
  // resulting kneeboard poses are equal (within 1e-5).   [Review Focus 5]
TEST(VRKneeboard, RecenterCapturesEyeHeight)
  // Recenter at hmd y = 1.2 vs y = 1.8: kneeboard y differs by 0.6.
TEST(VRKneeboard, HeadMovementWithoutRecenterDoesNotMoveKneeboard)
  // First call at (0,1.7,0); later call at (0.5,1.7,0), same mRecenterCount -> pose unchanged.
```

- [ ] **Step 2: Build and run**

Run: `ctest --test-dir build-tests -C Debug -R VRKneeboard --output-on-failure`
Expected: `0 tests failed out of 5`

- [ ] **Step 3: Commit**

```bash
git add src/tests
git commit -m "test: characterize VRKneeboard recenter and pose math"
```

---

### Task 4: SHM protocol characterization

**Files:**
- Create: `src/tests/SHM_test.cpp`
- Modify: `src/tests/CMakeLists.txt` (link `OpenKneeboard-SHM`)

**Interfaces:**
- Consumes: `SHM::Writer(uint64_t gpuLuid)`, `Writer::SubmitEmptyFrame()`, `Writer::Detach()`, `Writer::lock()/try_lock()/unlock()`; `SHM::Reader(ConsumerKind, uint64_t gpuLUID)` (abstract: test subclass implements `OnSessionChanged()` as a counter); `Reader::MaybeGet() -> std::expected<Frame, Frame::Error>`; `Reader::operator bool`.
- Produces: fixture `SHMTest` whose `SetUp()` constructs a probe `Reader` and calls `GTEST_SKIP() << "OpenKneeboard appears to be running; close it to run SHM tests"` if `static_cast<bool>(probe)` is true. [Review Focus 1]

- [ ] **Step 1: Write the tests**

```cpp
TEST_F(SHMTest, NoWriterMeansNoFeeder)
  // Reader only -> MaybeGet().error() == Frame::Error::NoFeeder; bool(reader) == false.
TEST_F(SHMTest, EmptyFrameCharacterization)
  // Writer(luid 1) + SubmitEmptyFrame(); Reader(Viewer, luid 1).MaybeGet()
  // -> record observed error (expected NoLayers or NoFeeder).
TEST_F(SHMTest, GpuMismatchCharacterization)
  // Writer(luid 1) + SubmitEmptyFrame(); Reader(Viewer, luid 2).MaybeGet()
  // -> record observed error (expected IncorrectGPU if the feeder flag is set).
TEST_F(SHMTest, DetachedWriterMeansNoFeeder)
  // Writer submits, then Detach() -> reader sees NoFeeder.
TEST_F(SHMTest, ReaderBlocksWhileWriterHoldsLock)
  // Writer.lock() on thread A; reader MaybeGet() on thread B does not return
  // within 200 ms; returns after unlock(). Pins spec Section 2.2 item 1
  // (the INFINITE wait) as current behavior; Phase 2 inverts this test.
```
For the characterization tests, assert the observed values and note them for Task 8.

- [ ] **Step 2: Build and run** (OpenKneeboard closed)

Run: `ctest --test-dir build-tests -C Debug -R SHM --output-on-failure`
Expected: `0 tests failed out of 5`. With OpenKneeboard running: 5 skipped, 0 failed.

- [ ] **Step 3: Commit**

```bash
git add src/tests
git commit -m "test: characterize SHM reader/writer protocol, including blocking lock"
```

---

### Task 5: Plugin manifest and VR settings JSON characterization

**Files:**
- Create: `src/tests/PluginManifest_test.cpp`, `src/tests/VRSettings_test.cpp`
- Create: `src/tests/data/plugin-v1-full.json`, `src/tests/data/plugin-v1-minimal.json` (copy the field set from `docs/api/web-dashboards/plugins.md`)
- Modify: `src/tests/CMakeLists.txt` (compile `src/app/app-common/Plugin.cpp` and `src/app/app-common/VRSettings.cpp` into the test target or a small static library `OpenKneeboard-Tests-AppCommonJson`, with whatever dependencies those two files need: `OpenKneeboard-json`, cppwinrt, `nlohmann_json`; copy `data/` next to the executable)

**Interfaces:**
- Consumes: `struct Plugin` and its JSON (de)serialization (`src/app/app-common/include/OpenKneeboard/Plugin.hpp`, `Plugin.cpp`); `VRSettings` JSON (`src/lib/include/OpenKneeboard/VRSettings.hpp`, `src/app/app-common/VRSettings.cpp`).

- [ ] **Step 1: Write the tests**

```cpp
TEST(PluginManifest, ParsesFullV1)
  // plugin-v1-full.json -> mID, Metadata fields, one WebBrowser TabType with
  // mURI and mInitialSize as in the file.
TEST(PluginManifest, AppliesDefaults)
  // plugin-v1-minimal.json -> mOKBMinimumVersion == "1.9",
  // WebBrowserArgs::mInitialSize == {1024, 768}.
TEST(PluginManifest, UnknownImplementationIsPreserved)
  // TabType with Implementation "SomethingNew" -> Implementation::Invalid with
  // InvalidImplementationArgs{mName == "SomethingNew"}.
TEST(PluginManifest, RoundTripsUnchanged)
  // parse(full) -> to_json -> parse == original (operator==).
TEST(VRSettings, DefaultRoundTrip)
  // VRSettings{} -> json -> VRSettings == default.
TEST(VRSettings, SparseJsonKeepsDefaults)
  // json with one changed field -> that field set, all others == default.
```

- [ ] **Step 2: Build and run**

Run: `ctest --test-dir build-tests -C Debug -R "PluginManifest|VRSettings" --output-on-failure`
Expected: `0 tests failed out of 6`

- [ ] **Step 3: Commit**

```bash
git add src/tests
git commit -m "test: characterize plugin v1 manifest and VR settings JSON"
```

---

### Task 6: Rust workspace and `okb-shm` constants crate

**Files:**
- Create: `rust/Cargo.toml` (workspace, `resolver = "3"`, `members = ["crates/*"]`, `[workspace.lints.rust] unsafe_code = "forbid"`, `[workspace.lints.clippy] all = "deny"`)
- Create: `rust/rust-toolchain.toml` (`channel = "<exact rustc version>"`, `components = ["clippy", "rustfmt", "llvm-tools-preview"]`)
- Create: `rust/crates/okb-shm/Cargo.toml`, `rust/crates/okb-shm/src/lib.rs`
- Create: `rust/crates/okb-shm/tests/matches_cpp.rs`
- Modify: `.gitignore` (add `rust/target/`)

**Interfaces:**
- Produces (Phase 1 builds on these): in `okb_shm`:
  `pub const SWAPCHAIN_LENGTH: usize = 2;`
  `pub const MAX_VIEW_COUNT: usize = 16;`
  each with a doc comment naming its C++ source (`Config::SHMSwapchainLength`, `MaxViewCount` in `src/lib/include/OpenKneeboard/config.in.hpp`).

- [ ] **Step 1: Write the failing test** `rust/crates/okb-shm/tests/matches_cpp.rs`

```rust
#[test]
fn constants_match_cpp_config() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../src/lib/include/OpenKneeboard/config.in.hpp"
    ))
    .expect("C++ config header readable");
    assert_eq!(cpp_const(&src, "SHMSwapchainLength"), okb_shm::SWAPCHAIN_LENGTH, "SHMSwapchainLength drifted");
    assert_eq!(cpp_const(&src, "MaxViewCount"), okb_shm::MAX_VIEW_COUNT, "MaxViewCount drifted");
}
```
`cpp_const(src, name) -> usize` finds the line containing `name =` and parses the integer before `;`; it panics with the name if not found. [Review Focus 3]

- [ ] **Step 2: Run it to verify it fails**

Run: `cd rust; cargo nextest run`
Expected: FAIL (constants not defined).

- [ ] **Step 3: Add the constants** in `rust/crates/okb-shm/src/lib.rs`

- [ ] **Step 4: Run tests, lints, and formatting**

Run: `cd rust; cargo nextest run; cargo clippy --all-targets; cargo fmt --check`
Expected: `1 test run: 1 passed`; clippy and fmt are clean.

- [ ] **Step 5: Verify that drift is caught**

Temporarily change `MaxViewCount = 16` to `17` in `config.in.hpp`; `cargo nextest run` fails with "MaxViewCount drifted"; revert.

- [ ] **Step 6: Commit**

```bash
git add rust .gitignore
git commit -m "feat(rust): add Cargo workspace and okb-shm wire constants checked against C++"
```

---

### Task 7: Coverage scripts, CI jobs, and the baseline numbers

**Files:**
- Create: `scripts/coverage-cpp.ps1` (params: `-BuildDir` default `build-tests`, `-Config` default `Debug`; runs `OpenCppCoverage --sources "$repo\src" --excluded_sources "$repo\src\tests" --excluded_sources "$repo\third-party" --cover_children --export_type "cobertura:$BuildDir\coverage-cpp.xml" -- ctest --test-dir $BuildDir -C $Config --output-on-failure`; then parses `line-rate`, `lines-covered`, `lines-valid` from the XML, prints `C++ line coverage: <pct>% (<covered>/<valid>)`, and exits 1 if `lines-covered` is 0) [Review Focus 4]
- Create: `scripts/coverage-rust.ps1` (runs `cargo llvm-cov nextest --workspace --cobertura --output-path target/coverage-rust.xml` in `rust/`, prints `Rust line coverage: <pct>%`)
- Modify: `.github/workflows/ci.yml`:
  - In the `Build` job, for `matrix.config == 'Debug'` only: add `-DOPENKNEEBOARD_BUILD_TESTS=ON` to Configure; after Build, install OpenCppCoverage (choco), run `scripts/coverage-cpp.ps1 -BuildDir ${{github.workspace}}/build`, upload `coverage-cpp.xml` as artifact `coverage-cpp`, and append the summary line to `$GITHUB_STEP_SUMMARY`.
  - New job `rust` (`runs-on: windows-2022`, no vcpkg): checkout, `dtolnay/rust-toolchain` reading `rust/rust-toolchain.toml`, `taiki-e/install-action` for `cargo-nextest,cargo-llvm-cov`, then `cargo clippy --all-targets`, `cargo fmt --check`, `scripts/coverage-rust.ps1`; upload `coverage-rust`; write the summary line.
- Create: `docs/internals/testing.md` (how to build and run both suites locally, the prerequisites list above, and a **Baseline** table)

- [ ] **Step 1: Run both scripts locally**

Run: `pwsh scripts/coverage-cpp.ps1; pwsh scripts/coverage-rust.ps1`
Expected: two summary lines with non-zero percentages; the C++ script exits 0.

- [ ] **Step 2: Fill in the Baseline table** in `docs/internals/testing.md`

| Measure | Value |
|---|---|
| Pre-Phase-0 automated tests | 0 (0% coverage) |
| C++ line coverage, `src/` (OpenCppCoverage, modules loaded by tests) | `<pct>% (<covered>/<valid>)` from Step 1 |
| C++ line coverage of all `src/` C++ lines (`<covered>` ÷ total lines in `src/**/*.cpp`) | `<pct>%` |
| Rust line coverage | `<pct>%` |

Count the total lines with `git ls-files 'src/**/*.cpp' | xargs cat | wc -l` and state the date and commit.

- [ ] **Step 3: Push the branch and verify CI**

Ask the user before pushing (the push goes outside the worktree). Expected: the `Build (Debug)` job shows the C++ coverage line in its summary; the `rust` job passes and shows the Rust coverage line; `Build (RelWithDebInfo)` is unchanged.

- [ ] **Step 4: Commit**

```bash
git add scripts/coverage-cpp.ps1 scripts/coverage-rust.ps1 .github/workflows/ci.yml docs/internals/testing.md
git commit -m "ci: run C++ and Rust tests with coverage; record baseline"
```

---

### Task 8: Parity contract for SHM and the OpenXR layer

**Files:**
- Create: `docs/internals/rust-port-parity-contract.md`

**Interfaces:**
- Consumes: observed values and quirks recorded in Tasks 1–5.
- Produces: the definition of done for Phase 1. Its section headings are referenced by Phase 1's plan: `## Unit of parity`, `## Must match exactly`, `## Acceptable differences`, `## How parity is measured`, `## Known quirks (preserve in Phase 1)`, `## Changes deferred to Phase 2`.

- [ ] **Step 1: Write the contract** with these decisions:
  - **Unit of parity:** (a) the SHM header and frame-slot byte layout, and the reader results (`Frame` fields or `Frame::Error`) for the same writer actions; (b) the OpenXR layer's appended `XrCompositionLayerQuad` list per `xrEndFrame`: count, order, pose, size, swapchain sub-image rect, and layer flags.
  - **Must match exactly:** byte layout of the SHM structures (`static_assert` sizes and offsets on the C++ side, mirrored by tests in Rust), `Frame::Error` cases, layer count and order, flags.
  - **Acceptable differences:** pose and size within 1e-5 m / 1e-5 rad; trace and log wording; timing.
  - **How parity is measured:** a differential harness feeds identical writer actions to the C++ and Rust implementations and compares results (SHM); a recorded-frame comparison under `hello_xr` + `test-feeder` (layer). Both are built in Phase 1.
  - **Known quirks:** every characterization value from Tasks 1–5 that looked surprising, one line each with the test name.
  - **Changes deferred to Phase 2:** the blocking `INFINITE` wait (`SHMTest.ReaderBlocksWhileWriterHoldsLock`), unbounded fence waits, `fatal()`/`OPENKNEEBOARD_ASSERT`/throwing `check_hresult` in in-game code (spec Section 2.2).

- [ ] **Step 2: Verify that every characterization test is cited**

Run: `grep -oE 'TEST(_F)?\([A-Za-z]+, *[A-Za-z]+Characterization' -r src/tests` and check that each name appears in the contract.
Expected: no uncited characterization test.

- [ ] **Step 3: Commit**

```bash
git add docs/internals/rust-port-parity-contract.md
git commit -m "docs: add Phase 1 parity contract for SHM and OpenXR layer"
```

---

## Later plans (not part of this plan)

One plan per spec phase, each written when the previous phase lands: Phase 1 (Rust SHM + layer, matching behavior), Phase 2 (isolation hardening), Phase 3 (plugin host), Phase 4 (passthrough mask, Quest + VD Method C first), Phase 5 (alignment + "move kneeboards with masks"), Phase 6 (Pimax/Varjo), Phase 7 (further migration).
