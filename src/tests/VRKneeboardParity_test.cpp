// OpenKneeboard
//
// Copyright (c) 2026 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.

// Differential tests: each scripted step runs through both the C++
// VRKneeboard and the Rust port in okb-vr (through okb-ffi), and the results
// must match.
//
// Gaze input focus writes the real SHM ActiveConsumers block, as in the game.
#include <OpenKneeboard/RayIntersectsRect.hpp>
#include <OpenKneeboard/SHM.hpp>
#include <OpenKneeboard/SHM/ActiveConsumers.hpp>
#include <OpenKneeboard/VRKneeboard.hpp>

#include <gtest/gtest.h>

#include <cmath>
#include <cstdint>
#include <memory>
#include <numbers>
#include <span>
#include <vector>

#include <okb_ffi.h>

using namespace OpenKneeboard;
using DirectX::SimpleMath::Quaternion;
using DirectX::SimpleMath::Vector2;
using DirectX::SimpleMath::Vector3;

static_assert(sizeof(OkbPose) == sizeof(VRKneeboard::Pose));
static_assert(sizeof(OkbVec2) == sizeof(Vector2));

namespace {

class TestableVRKneeboard : public VRKneeboard {
 public:
  using VRKneeboard::GetLayers;
};

struct KneeboardDeleter {
  void operator()(OkbVrKneeboard* p) const { okb_vr_kneeboard_free(p); }
};

constexpr float Epsilon = 1e-5f;
constexpr float Degrees = std::numbers::pi_v<float> / 180;
constexpr uint64_t NoCall = UINT64_MAX;

struct Step {
  SHM::Config mConfig {};
  std::vector<SHM::LayerConfig> mLayers;
  VRKneeboard::Pose mHmd {};
};

VRKneeboard::Pose Hmd(
  Vector3 position,
  float yawDeg = 0,
  float pitchDeg = 0,
  float rollDeg = 0) {
  return {
    .mPosition = position,
    .mOrientation = Quaternion::CreateFromYawPitchRoll(
      yawDeg * Degrees, pitchDeg * Degrees, rollDeg * Degrees),
  };
}

OkbVec3 ToOkb(const Vector3& v) {
  return {v.x, v.y, v.z};
}

OkbQuat ToOkb(const Quaternion& q) {
  return {q.x, q.y, q.z, q.w};
}

OkbPose ToOkb(const VRKneeboard::Pose& p) {
  return {ToOkb(p.mPosition), ToOkb(p.mOrientation)};
}

// The fixture layer of VRKneeboard_test.cpp.
SHM::LayerConfig DefaultLayer(uint64_t id) {
  SHM::LayerConfig layer {};
  layer.mLayerID = id;
  layer.mVREnabled = true;
  layer.mVR.mPhysicalSize = {0.2f, 0.3f};
  return layer;
}

// 1 m x 1 m, 1 m straight ahead of the first-frame HMD at (0, 1.7, 0).
SHM::LayerConfig AheadLayer(uint64_t id, bool lookable = true) {
  SHM::LayerConfig layer {};
  layer.mLayerID = id;
  layer.mVREnabled = true;
  layer.mVR.mPose = {0, 0, -1, 0, 0, 0};
  layer.mVR.mPhysicalSize = {1, 1};
  if (!lookable) {
    layer.mVR.mGazeTargetScale = {0, 0};
  }
  return layer;
}

SHM::Config GazeConfig(uint64_t globalInputLayerID, bool focus = true) {
  SHM::Config config {};
  config.mGlobalInputLayerID = globalInputLayerID;
  config.mVR.mEnableGazeInputFocus = focus;
  return config;
}

void ExpectNear(const VRKneeboard::Pose& cpp, const OkbPose& rust) {
  EXPECT_NEAR(cpp.mPosition.x, rust.position.x, Epsilon);
  EXPECT_NEAR(cpp.mPosition.y, rust.position.y, Epsilon);
  EXPECT_NEAR(cpp.mPosition.z, rust.position.z, Epsilon);

  // q and -q are the same rotation
  const auto& q = cpp.mOrientation;
  const auto& r = rust.orientation;
  const float sign =
    (q.x * r.x + q.y * r.y + q.z * r.z + q.w * r.w) < 0 ? -1.0f : 1.0f;
  EXPECT_NEAR(q.x, sign * r.x, Epsilon);
  EXPECT_NEAR(q.y, sign * r.y, Epsilon);
  EXPECT_NEAR(q.z, sign * r.z, Epsilon);
  EXPECT_NEAR(q.w, sign * r.w, Epsilon);
}

void RunScript(std::span<const Step> steps) {
  TestableVRKneeboard cpp;
  std::unique_ptr<OkbVrKneeboard, KneeboardDeleter> rust {
    okb_vr_kneeboard_new()};
  ASSERT_NE(rust, nullptr);

  for (std::size_t i = 0; i < steps.size(); ++i) {
    SCOPED_TRACE(testing::Message() << "step " << i);
    const auto& step = steps[i];
    ASSERT_LE(step.mLayers.size(), 16);

    // A sentinel distinguishes "no call" from `SetActiveInGameViewID({})`.
    const uint64_t sentinel = 0x5E00'0000 + i;
    SHM::ActiveConsumers::SetActiveInGameViewID(sentinel);

    const auto cppLayers = cpp.GetLayers(step.mConfig, step.mLayers, step.mHmd);
    const auto cppConsumers = SHM::ActiveConsumers::Get();

    OkbVrLayerOut out[16] {};
    uint32_t outCount = 0;
    uint64_t activeView = 0;
    okb_vr_kneeboard_get_layers(
      rust.get(),
      reinterpret_cast<const OkbConfig*>(&step.mConfig),
      reinterpret_cast<const OkbLayerConfig*>(step.mLayers.data()),
      step.mLayers.size(),
      ToOkb(step.mHmd),
      out,
      &outCount,
      &activeView);

    ASSERT_EQ(cppLayers.size(), outCount);
    for (std::size_t j = 0; j < outCount; ++j) {
      SCOPED_TRACE(testing::Message() << "layer " << j);
      const auto& c = cppLayers.at(j);
      const auto& r = out[j];
      EXPECT_EQ(c.mLayerConfig - step.mLayers.data(), r.layer_index);
      const auto& params = c.mRenderParameters;
      ExpectNear(params.mKneeboardPose, r.kneeboard_pose);
      EXPECT_NEAR(params.mKneeboardSize.x, r.kneeboard_size.x, Epsilon);
      EXPECT_NEAR(params.mKneeboardSize.y, r.kneeboard_size.y, Epsilon);
      EXPECT_EQ(params.mKneeboardOpacity, r.kneeboard_opacity);
      EXPECT_EQ(params.mIsLookingAtKneeboard, r.is_looking_at_kneeboard);
    }

    // Apply the Rust result as `SetActiveInGameViewID` would.
    uint64_t rustActive = sentinel;
    uint64_t rustGlobal = sentinel;
    if (activeView != NoCall) {
      rustActive = activeView;
      if (activeView) {
        rustGlobal = activeView;
      }
    }
    EXPECT_EQ(cppConsumers.mActiveViewID, rustActive);
    EXPECT_EQ(cppConsumers.mGlobalInputViewID, rustGlobal);
  }
}

}// namespace

TEST(VRKneeboardParity, NoRecenter) {
  SHM::Config config {};
  config.mVR.mEnableGazeInputFocus = false;
  const std::vector layers {DefaultLayer(1)};
  const Step steps[] {
    {config, layers, Hmd({0, 1.7f, 0})},
    {config, layers, Hmd({0, 1.7f, 0})},
    {config, layers, Hmd({0.5f, 1.6f, -0.2f}, 30)},
    {config, layers, Hmd({1, 1.7f, 2}, 45, 10, 5)},
  };
  RunScript(steps);
}

TEST(VRKneeboardParity, RecenterMidScript) {
  auto tilted = DefaultLayer(2);
  tilted.mVR.mPose = {-0.2f, -0.5f, -0.6f, 0.3f, 0.7f, -0.4f};
  const std::vector layers {DefaultLayer(1), tilted};

  SHM::Config rc0 {};
  auto rc1 = rc0;
  rc1.mVR.mRecenterCount = 1;
  auto rc2 = rc0;
  rc2.mVR.mRecenterCount = 2;

  const Step steps[] {
    {rc0, layers, Hmd({0, 1.7f, 0})},
    {rc0, layers, Hmd({0.3f, 1.6f, 0.2f}, 30)},
    {rc1, layers, Hmd({1, 1.5f, 2}, 90, 20)},
    {rc1, layers, Hmd({1.2f, 1.4f, 2.1f}, 80, 10)},
    {rc2, layers, Hmd({-0.5f, 1.8f, 0.4f}, -135, -30, 10)},
    {rc2, layers, Hmd({-0.5f, 1.8f, 0.4f}, -135, -30, 10)},
  };
  RunScript(steps);
}

TEST(VRKneeboardParity, GazeZoomAndOpacity) {
  auto layer = AheadLayer(1);
  layer.mVR.mOpacity = {.mNormal = 0.5f, .mGaze = 0.9f};
  layer.mVR.mZoomScale = 1.5f;
  const std::vector layers {layer};

  auto noGazeZoom = layer;
  noGazeZoom.mVR.mEnableGazeZoom = false;
  auto tinyTarget = layer;
  tinyTarget.mVR.mGazeTargetScale = {0.05f, 1};
  auto halfTarget = layer;
  halfTarget.mVR.mGazeTargetScale = {0.5f, 0.5f};

  const auto config = GazeConfig(0, false);
  auto forceZoom = config;
  forceZoom.mVR.mForceZoom = true;

  const Step steps[] {
    // Looking: zoomed, gaze opacity
    {config, layers, Hmd({0, 1.7f, 0})},
    // 0.6 m off center: outside the normal half-width (0.5), inside the
    // zoomed one (0.75), which applies while still looking
    {config, layers, Hmd({0.6f, 1.7f, 0})},
    // Look away
    {config, layers, Hmd({0, 1.7f, 0}, 90)},
    // Back at 0.6 m: not looking, so the normal size is the target
    {config, layers, Hmd({0.6f, 1.7f, 0})},
    {forceZoom, layers, Hmd({0.6f, 1.7f, 0})},
    {config, {noGazeZoom}, Hmd({0, 1.7f, 0})},
    {config, {tinyTarget}, Hmd({0, 1.7f, 0})},
    {config, {halfTarget}, Hmd({0.3f, 1.7f, 0})},
    {config, {halfTarget}, Hmd({0.2f, 1.7f, 0})},
  };
  RunScript(steps);
}

TEST(VRKneeboardParity, GazeFocusGlobalLayer) {
  const std::vector layers {AheadLayer(1), AheadLayer(2)};
  const auto config = GazeConfig(2);
  const Step steps[] {
    {config, layers, Hmd({0, 1.7f, 0})},
    {config, layers, Hmd({0, 1.7f, 0}, 90)},
    {config, layers, Hmd({0, 1.7f, 0})},
    {GazeConfig(1), layers, Hmd({0, 1.7f, 0})},
    {GazeConfig(2, false), layers, Hmd({0, 1.7f, 0})},
  };
  RunScript(steps);
}

TEST(VRKneeboardParity, GazeFocusFallback) {
  // The first layer with the global ID is not lookable; a later duplicate is,
  // and the reverse scan skips it.
  const std::vector layers {
    AheadLayer(1),
    AheadLayer(3, false),
    AheadLayer(2),
    AheadLayer(3),
  };
  const Step steps[] {
    {GazeConfig(3), layers, Hmd({0, 1.7f, 0})},
    {GazeConfig(7), layers, Hmd({0, 1.7f, 0})},
    {GazeConfig(3), layers, Hmd({0, 1.7f, 0}, 180)},
    {GazeConfig(3), {AheadLayer(1, false)}, Hmd({0, 1.7f, 0})},
  };
  RunScript(steps);
}

TEST(VRKneeboardParity, MixedVrEnabled) {
  std::vector layers {
    AheadLayer(1),
    AheadLayer(2),
    DefaultLayer(3),
    AheadLayer(4),
  };
  layers[1].mVREnabled = false;
  auto flipped = layers;
  for (auto& layer: flipped) {
    layer.mVREnabled = !layer.mVREnabled;
  }
  std::vector<SHM::LayerConfig> noneEnabled = layers;
  for (auto& layer: noneEnabled) {
    layer.mVREnabled = false;
  }

  const Step steps[] {
    {GazeConfig(2), layers, Hmd({0, 1.7f, 0})},
    {GazeConfig(2), flipped, Hmd({0, 1.7f, 0})},
    {GazeConfig(4), flipped, Hmd({0, 1.7f, 0})},
    {GazeConfig(1), noneEnabled, Hmd({0, 1.7f, 0})},
    {GazeConfig(1), {}, Hmd({0, 1.7f, 0})},
  };
  RunScript(steps);
}

TEST(VRKneeboardParity, RayIntersectsRect) {
  const Vector2 size {1, 1};
  for (const float x: {0.f, 0.3f, 0.49f, 0.51f, -0.7f}) {
    for (const float rayYaw: {0.f, 10.f, 180.f}) {
      for (const float rectYaw: {0.f, 45.f, 60.f, 90.f}) {
        for (const float rectZ: {-2.f, 2.f}) {
          SCOPED_TRACE(
            testing::Message() << x << " " << rayYaw << " " << rectYaw << " "
                               << rectZ);
          const Vector3 origin {x, 0, 0};
          const Vector3 center {0, 0, rectZ};
          const auto rayQ =
            Quaternion::CreateFromYawPitchRoll(rayYaw * Degrees, 0, 0);
          const auto rectQ =
            Quaternion::CreateFromYawPitchRoll(rectYaw * Degrees, 0, 0);
          EXPECT_EQ(
            RayIntersectsRect(origin, rayQ, center, rectQ, size),
            okb_ray_intersects_rect(
              ToOkb(origin),
              ToOkb(rayQ),
              ToOkb(center),
              ToOkb(rectQ),
              {size.x, size.y}));
        }
      }
    }
  }
}
