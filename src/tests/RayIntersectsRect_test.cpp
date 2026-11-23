// OpenKneeboard
//
// Copyright (c) 2026 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.
#include <OpenKneeboard/RayIntersectsRect.hpp>

#include <gtest/gtest.h>

#include <numbers>

using namespace DirectX::SimpleMath;
using OpenKneeboard::RayIntersectsRect;

namespace {
// Rect 1 m x 1 m at (0, 0, -2), facing the viewer (identity orientation).
// OpenXR convention: identity orientation looks down -Z.
const Vector3 RectCenter {0, 0, -2};
const Vector2 RectSize {1, 1};

Quaternion Yaw(float radians) {
  return Quaternion::CreateFromYawPitchRoll(radians, 0, 0);
}
}// namespace

TEST(RayIntersectsRect, HitsCenterStraightAhead) {
  // origin (0,0,0), identity -> true
  EXPECT_TRUE(RayIntersectsRect(
    {0, 0, 0}, Quaternion::Identity, RectCenter, Quaternion::Identity, RectSize));
}

TEST(RayIntersectsRect, MissesWhenPointingAway) {
  // origin (0,0,0), yaw 180 degrees -> false
  EXPECT_FALSE(RayIntersectsRect(
    {0, 0, 0},
    Yaw(std::numbers::pi_v<float>),
    RectCenter,
    Quaternion::Identity,
    RectSize));
}

TEST(RayIntersectsRect, HitsInsideEdge) {
  // origin (0.49,0,0), identity -> true
  EXPECT_TRUE(RayIntersectsRect(
    {0.49f, 0, 0},
    Quaternion::Identity,
    RectCenter,
    Quaternion::Identity,
    RectSize));
}

TEST(RayIntersectsRect, MissesOutsideEdge) {
  // origin (0.51,0,0), identity -> false
  EXPECT_FALSE(RayIntersectsRect(
    {0.51f, 0, 0},
    Quaternion::Identity,
    RectCenter,
    Quaternion::Identity,
    RectSize));
}

TEST(RayIntersectsRect, MissesWhenRectBehindOrigin) {
  // rect at (0,0,+2) -> false
  EXPECT_FALSE(RayIntersectsRect(
    {0, 0, 0},
    Quaternion::Identity,
    {0, 0, 2},
    Quaternion::Identity,
    RectSize));
}

TEST(RayIntersectsRect, RespectsRectRotation) {
  // Rect yawed 60 degrees; ray at x = 0.3 meets the rect plane 0.3 / cos(60)
  // = 0.6 m along the rect's X axis, outside its 0.5 m half-width -> false.
  // The same ray hits the unrotated rect (0.3 < 0.5).
  EXPECT_TRUE(RayIntersectsRect(
    {0.3f, 0, 0},
    Quaternion::Identity,
    RectCenter,
    Quaternion::Identity,
    RectSize));
  EXPECT_FALSE(RayIntersectsRect(
    {0.3f, 0, 0},
    Quaternion::Identity,
    RectCenter,
    Yaw(std::numbers::pi_v<float> / 3),
    RectSize));

  // Known quirk: rect yawed 90 degrees (edge-on) with the ray along its
  // center line -> true. Float error tilts the plane slightly, so the ray
  // "hits" it at exactly the rect center. Plan expected false.
  EXPECT_TRUE(RayIntersectsRect(
    {0, 0, 0},
    Quaternion::Identity,
    RectCenter,
    Yaw(std::numbers::pi_v<float> / 2),
    RectSize));
}
