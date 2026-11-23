// OpenKneeboard
//
// Copyright (c) 2026 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.
#include <OpenKneeboard/SHM.hpp>
#include <OpenKneeboard/VRKneeboard.hpp>

#include <gtest/gtest.h>

#include <numbers>

using namespace OpenKneeboard;
using DirectX::SimpleMath::Quaternion;
using DirectX::SimpleMath::Vector3;

namespace {

class TestableVRKneeboard : public VRKneeboard {
 public:
  using VRKneeboard::GetLayers;
};

constexpr float Epsilon = 1e-5f;
constexpr float Degrees = std::numbers::pi_v<float> / 180;

struct Fixture {
  SHM::Config mConfig {};
  SHM::LayerConfig mLayer {};

  Fixture() {
    // Gaze input focus writes to the real SHM ActiveConsumers block; keep
    // these tests pure.
    mConfig.mVR.mEnableGazeInputFocus = false;
    mLayer.mLayerID = 1;
    mLayer.mVREnabled = true;
    mLayer.mVR.mPose = {};// VRPose defaults
    mLayer.mVR.mPhysicalSize = {0.2f, 0.3f};
  }

  VRKneeboard::Pose Get(TestableVRKneeboard& kb, const VRKneeboard::Pose& hmd) {
    const auto layers = kb.GetLayers(mConfig, {&mLayer, 1}, hmd);
    EXPECT_EQ(layers.size(), 1);
    return layers.at(0).mRenderParameters.mKneeboardPose;
  }

  // Bump mRecenterCount and recenter at `hmd`.
  VRKneeboard::Pose RecenterAt(
    TestableVRKneeboard& kb,
    const VRKneeboard::Pose& hmd) {
    ++mConfig.mVR.mRecenterCount;
    return Get(kb, hmd);
  }
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

void ExpectNear(const Vector3& a, const Vector3& b) {
  EXPECT_NEAR(a.x, b.x, Epsilon);
  EXPECT_NEAR(a.y, b.y, Epsilon);
  EXPECT_NEAR(a.z, b.z, Epsilon);
}

void ExpectNear(const VRKneeboard::Pose& a, const VRKneeboard::Pose& b) {
  ExpectNear(a.mPosition, b.mPosition);
  // q and -q are the same rotation
  const auto dot = std::abs(a.mOrientation.Dot(b.mOrientation));
  EXPECT_NEAR(dot, 1.0f, Epsilon);
}

}// namespace

TEST(VRKneeboard, PoseIsStableWithoutRecenter) {
  // Same hmd pose, two calls, same mRecenterCount -> identical kneeboard pose.
  Fixture f;
  TestableVRKneeboard kb;
  const auto hmd = Hmd({0, 1.7f, 0});
  const auto a = f.Get(kb, hmd);
  const auto b = f.Get(kb, hmd);
  EXPECT_EQ(a.mPosition, b.mPosition);
  EXPECT_EQ(a.mOrientation, b.mOrientation);
}

TEST(VRKneeboard, RecenterFollowsHeadsetTranslationInXZ) {
  // Recenter at hmd (0,1.7,0); then bump mRecenterCount and recenter at
  // (1,1.7,2). Kneeboard position delta == (1, 0, 2).
  Fixture f;
  TestableVRKneeboard kb;
  const auto a = f.RecenterAt(kb, Hmd({0, 1.7f, 0}));
  const auto b = f.RecenterAt(kb, Hmd({1, 1.7f, 2}));
  ExpectNear(b.mPosition - a.mPosition, {1, 0, 2});
}

TEST(VRKneeboard, RecenterUsesYawOnly) {
  // Recenter with hmd yaw 90 vs yaw 90 + pitch 30 + roll 20: resulting
  // kneeboard poses are equal. A tilted head must not tilt kneeboards.
  Fixture f;
  TestableVRKneeboard yawOnly;
  TestableVRKneeboard tilted;
  const auto a = f.RecenterAt(yawOnly, Hmd({0, 1.7f, 0}, 90));
  --f.mConfig.mVR.mRecenterCount;
  const auto b = f.RecenterAt(tilted, Hmd({0, 1.7f, 0}, 90, 30, 20));
  ExpectNear(a, b);
}

TEST(VRKneeboard, RecenterCapturesEyeHeight) {
  // Recenter at hmd y = 1.2 vs y = 1.8: kneeboard y differs by 0.6.
  Fixture f;
  TestableVRKneeboard kb;
  const auto low = f.RecenterAt(kb, Hmd({0, 1.2f, 0}));
  const auto high = f.RecenterAt(kb, Hmd({0, 1.8f, 0}));
  EXPECT_NEAR(high.mPosition.y - low.mPosition.y, 0.6f, Epsilon);
}

TEST(VRKneeboard, HeadMovementWithoutRecenterDoesNotMoveKneeboard) {
  // First call at (0,1.7,0); later call at (0.5,1.7,0), same mRecenterCount
  // -> pose unchanged.
  Fixture f;
  TestableVRKneeboard kb;
  const auto a = f.Get(kb, Hmd({0, 1.7f, 0}));
  const auto b = f.Get(kb, Hmd({0.5f, 1.7f, 0}));
  ExpectNear(a, b);
}

TEST(VRKneeboard, FirstFrameDoesNotRecenter) {
  // No recenter happens until mRecenterCount changes from 0: a first frame at
  // (1,1.7,2) yaw 45 places the kneeboard exactly like a first frame at the
  // origin (same eye height, so Y matches too). Known quirk in the parity
  // contract.
  Fixture f;
  TestableVRKneeboard atOrigin;
  TestableVRKneeboard offset;
  const auto a = f.Get(atOrigin, Hmd({0, 1.7f, 0}));
  const auto b = f.Get(offset, Hmd({1, 1.7f, 2}, 45));
  ExpectNear(a, b);
}
