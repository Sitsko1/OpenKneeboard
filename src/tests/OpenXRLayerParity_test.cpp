// OpenKneeboard
//
// Copyright (c) 2026 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.

// Characterizes the C++ OpenXR layer through the harness in
// OpenXRLayerHarness.hpp, then checks the Rust layer records the same frames.

#include "OpenXRLayerHarness.hpp"

#include <OpenKneeboard/SpriteAtlas.hpp>

#include <gtest/gtest.h>

#include <DirectXMath.h>

#include <algorithm>
#include <cstdlib>
#include <numbers>

using namespace OpenKneeboard;
using namespace OpenKneeboard::Tests;

namespace {

const std::filesystem::path CppLayer {OKB_TEST_CPP_LAYER_DLL};
const std::filesystem::path RustLayer {OKB_TEST_RUST_LAYER_DLL};

SHM::LayerConfig MakeLayer(uint64_t id, const PixelRect& location) {
  SHM::LayerConfig layer {.mLayerID = id, .mVREnabled = true};
  layer.mVR.mPhysicalSize = {0.15f, 0.25f};
  layer.mVR.mLocationOnTexture = location;
  return layer;
}

SHM::Config MakeConfig(const HarnessScript& script) {
  return {.mTextureSize = script.mSourceTextureSize};
}

#define SKIP_IF_UNSUPPORTED() \
  if (!HarnessSupported()) { \
    GTEST_SKIP() << HarnessSkipMessage; \
  } \
  ::testing::Test::RecordProperty("d3d11_adapter", HarnessAdapterDescription());

}// namespace

TEST(CppLayer, SingleLayerAppendsOneQuad) {
  SKIP_IF_UNSUPPORTED();
  HarnessScript script;
  const PixelRect location {{0, 0}, {512, 768}};
  script.mSteps.push_back({
    .mConfig = MakeConfig(script),
    .mLayers = {MakeLayer(1, location)},
  });

  const auto frames = RunHarness(CppLayer, script);
  ASSERT_EQ(frames.size(), 1);
  const auto& frame = frames.front();
  EXPECT_TRUE(frame.mGameLayersForwardedUnchanged);
  EXPECT_EQ(frame.mLayerCount, 2);
  ASSERT_EQ(frame.mQuads.size(), 1);
  const auto& quad = frame.mQuads.front();
  EXPECT_EQ(quad.mFlags, XR_COMPOSITION_LAYER_BLEND_TEXTURE_SOURCE_ALPHA_BIT);
  EXPECT_EQ(quad.mEye, XR_EYE_VISIBILITY_BOTH);
  EXPECT_EQ(quad.mSpace, "LOCAL");
  EXPECT_EQ(quad.mSwapchainOrdinal, 0);

  PixelSize sizes[] {location.mSize};
  const auto rect = SpriteAtlas {sizes}.GetRect(0);
  EXPECT_EQ(quad.mImageRect.offset.x, static_cast<int>(rect.mOffset.mX));
  EXPECT_EQ(quad.mImageRect.offset.y, static_cast<int>(rect.mOffset.mY));
  EXPECT_EQ(quad.mImageRect.extent.width, static_cast<int>(rect.mSize.mWidth));
  EXPECT_EQ(
    quad.mImageRect.extent.height, static_cast<int>(rect.mSize.mHeight));

  ASSERT_NE(frame.mSwapchainWidth, 0);
  ASSERT_FALSE(frame.mSwapchainPixels.empty());
  bool anyNonZero = false;
  for (int y = quad.mImageRect.offset.y;
       y < quad.mImageRect.offset.y + quad.mImageRect.extent.height;
       ++y) {
    const auto row = frame.mSwapchainPixels.data()
      + (y * frame.mSwapchainWidth + quad.mImageRect.offset.x) * 4;
    anyNonZero = anyNonZero
      || std::any_of(row, row + quad.mImageRect.extent.width * 4, [](auto b) {
                   return b != 0;
                 });
  }
  EXPECT_TRUE(anyNonZero);

  // 1:1 copy of the feeder pattern, BGRA, at opacity 1: texel (100, 200).
  const auto texel = frame.mSwapchainPixels.data()
    + ((quad.mImageRect.offset.y + 200) * frame.mSwapchainWidth
       + quad.mImageRect.offset.x + 100)
      * 4;
  EXPECT_NEAR(texel[0], (100 + 200) % 239, 1);
  EXPECT_NEAR(texel[1], 200 % 241, 1);
  EXPECT_NEAR(texel[2], 100 % 251, 1);
  EXPECT_NEAR(texel[3], 255, 1);
}

TEST(CppLayer, TruncatesToRuntimeMax) {
  SKIP_IF_UNSUPPORTED();
  HarnessScript script {.mMaxLayerCount = 3};
  script.mSteps.push_back({
    .mConfig = MakeConfig(script),
    .mLayers =
      {
        MakeLayer(1, {{0, 0}, {256, 256}}),
        MakeLayer(2, {{256, 0}, {256, 256}}),
        MakeLayer(3, {{512, 0}, {256, 256}}),
      },
    .mGameLayerCount = 2,
  });

  const auto frames = RunHarness(CppLayer, script);
  ASSERT_EQ(frames.size(), 1);
  EXPECT_TRUE(frames.front().mGameLayersForwardedUnchanged);
  EXPECT_EQ(frames.front().mLayerCount, 3);
  EXPECT_EQ(frames.front().mQuads.size(), 1);
}

TEST(CppLayer, NoFeederPassesThrough) {
  SKIP_IF_UNSUPPORTED();
  HarnessScript script;
  script.mSteps.push_back({.mDetach = true});

  const auto frames = RunHarness(CppLayer, script);
  ASSERT_EQ(frames.size(), 1);
  EXPECT_TRUE(frames.front().mGameLayersForwardedUnchanged);
  EXPECT_EQ(frames.front().mLayerCount, 1);
  EXPECT_TRUE(frames.front().mQuads.empty());
  EXPECT_TRUE(frames.front().mSwapchainPixels.empty());
}

///// C++/Rust parity /////

namespace {

constexpr float Epsilon = 1e-5f;

XrPosef Hmd(float x, float y, float z, float yawDegrees = 0) {
  const auto q = DirectX::XMQuaternionRotationRollPitchYaw(
    0, yawDegrees * std::numbers::pi_v<float> / 180, 0);
  DirectX::XMFLOAT4 o;
  DirectX::XMStoreFloat4(&o, q);
  return {.orientation = {o.x, o.y, o.z, o.w}, .position = {x, y, z}};
}

// 1 m x 1 m, `x` m right of and 1 m ahead of an HMD at (0, 1.7, 0).
SHM::LayerConfig AheadLayer(uint64_t id, float x, const PixelRect& location) {
  auto layer = MakeLayer(id, location);
  layer.mVR.mPose = {x, 0, -1, 0, 0, 0};
  layer.mVR.mPhysicalSize = {1, 1};
  layer.mVR.mOpacity = {.mNormal = 0.5f, .mGaze = 1.0f};
  return layer;
}

void ExpectPixelsNear(
  const std::vector<uint8_t>& cpp,
  const std::vector<uint8_t>& rust) {
  ASSERT_EQ(cpp.size(), rust.size());
  size_t mismatches = 0;
  for (size_t i = 0; i < cpp.size(); ++i) {
    if (std::abs(int {cpp[i]} - int {rust[i]}) > 1 && ++mismatches <= 5) {
      ADD_FAILURE() << "byte " << i << " (pixel " << i / 4 << ", channel "
                    << i % 4 << "): C++ " << int {cpp[i]} << ", Rust "
                    << int {rust[i]};
    }
  }
  EXPECT_EQ(mismatches, 0) << "bytes differing by more than 1";
}

void ExpectQuadNear(const RecordedQuad& cpp, const RecordedQuad& rust) {
  EXPECT_NEAR(cpp.mPose.position.x, rust.mPose.position.x, Epsilon);
  EXPECT_NEAR(cpp.mPose.position.y, rust.mPose.position.y, Epsilon);
  EXPECT_NEAR(cpp.mPose.position.z, rust.mPose.position.z, Epsilon);
  // q and -q are the same rotation
  const auto& q = cpp.mPose.orientation;
  const auto& r = rust.mPose.orientation;
  const float sign =
    (q.x * r.x + q.y * r.y + q.z * r.z + q.w * r.w) < 0 ? -1.0f : 1.0f;
  EXPECT_NEAR(q.x, sign * r.x, Epsilon);
  EXPECT_NEAR(q.y, sign * r.y, Epsilon);
  EXPECT_NEAR(q.z, sign * r.z, Epsilon);
  EXPECT_NEAR(q.w, sign * r.w, Epsilon);
  EXPECT_NEAR(cpp.mSize.width, rust.mSize.width, Epsilon);
  EXPECT_NEAR(cpp.mSize.height, rust.mSize.height, Epsilon);

  EXPECT_EQ(cpp.mSwapchainOrdinal, rust.mSwapchainOrdinal);
  EXPECT_EQ(cpp.mImageRect.offset.x, rust.mImageRect.offset.x);
  EXPECT_EQ(cpp.mImageRect.offset.y, rust.mImageRect.offset.y);
  EXPECT_EQ(cpp.mImageRect.extent.width, rust.mImageRect.extent.width);
  EXPECT_EQ(cpp.mImageRect.extent.height, rust.mImageRect.extent.height);
  EXPECT_EQ(cpp.mImageArrayIndex, rust.mImageArrayIndex);
  EXPECT_EQ(cpp.mFlags, rust.mFlags);
  EXPECT_EQ(cpp.mEye, rust.mEye);
  EXPECT_EQ(cpp.mSpace, rust.mSpace);
}

// Runs the script through both layers; returns the C++ frames for any
// script-specific checks.
std::vector<RecordedFrame> ExpectParity(const HarnessScript& script) {
  const auto cpp = RunHarness(CppLayer, script);
  const auto rust = RunHarness(RustLayer, script);
  EXPECT_EQ(cpp.size(), script.mSteps.size());
  EXPECT_EQ(cpp.size(), rust.size());
  for (size_t i = 0; i < std::min(cpp.size(), rust.size()); ++i) {
    SCOPED_TRACE(testing::Message() << "frame " << i);
    const auto& c = cpp[i];
    const auto& r = rust[i];
    EXPECT_EQ(c.mGameLayersForwardedUnchanged, r.mGameLayersForwardedUnchanged);
    EXPECT_EQ(c.mLayerCount, r.mLayerCount);
    EXPECT_EQ(c.mQuads.size(), r.mQuads.size());
    for (size_t q = 0; q < std::min(c.mQuads.size(), r.mQuads.size()); ++q) {
      SCOPED_TRACE(testing::Message() << "quad " << q);
      ExpectQuadNear(c.mQuads[q], r.mQuads[q]);
    }
    EXPECT_EQ(c.mSwapchainWidth, r.mSwapchainWidth);
    EXPECT_EQ(c.mSwapchainFormat, r.mSwapchainFormat);
    ExpectPixelsNear(c.mSwapchainPixels, r.mSwapchainPixels);
    EXPECT_EQ(c.mActiveInGameViewID, r.mActiveInGameViewID);
  }
  return cpp;
}

}// namespace

TEST(RustLayerParity, SingleLayer) {
  SKIP_IF_UNSUPPORTED();
  HarnessScript script;
  const HarnessStep step {
    .mConfig = MakeConfig(script),
    .mLayers = {MakeLayer(1, {{0, 0}, {512, 768}})},
  };
  // Twice: the second frame reuses the swapchain and acquires the next image.
  script.mSteps = {step, step};
  const auto cpp = ExpectParity(script);
  ASSERT_EQ(cpp.size(), 2);
  EXPECT_EQ(cpp[1].mQuads.size(), 1);
}

TEST(RustLayerParity, MultiLayer) {
  SKIP_IF_UNSUPPORTED();
  HarnessScript script;
  auto config = MakeConfig(script);
  config.mVR.mEnableGazeInputFocus = false;
  script.mSteps.push_back({
    .mConfig = config,
    .mLayers =
      {
        AheadLayer(1, -1.5f, {{0, 0}, {300, 400}}),
        AheadLayer(2, 0, {{300, 0}, {500, 250}}),
        AheadLayer(3, 1.5f, {{0, 400}, {600, 600}}),
      },
  });
  const auto cpp = ExpectParity(script);
  ASSERT_EQ(cpp.size(), 1);
  EXPECT_EQ(cpp[0].mQuads.size(), 3);
  // Gaze input focus off: SetActiveInGameViewID is not called.
  EXPECT_EQ(cpp[0].mActiveInGameViewID, ActiveInGameViewSentinel(0));
}

TEST(RustLayerParity, MoreLayersThanRuntimeAllows) {
  SKIP_IF_UNSUPPORTED();
  HarnessScript script {.mMaxLayerCount = 4};
  script.mSteps.push_back({
    .mConfig = MakeConfig(script),
    .mLayers =
      {
        MakeLayer(1, {{0, 0}, {256, 256}}),
        MakeLayer(2, {{256, 0}, {256, 300}}),
        MakeLayer(3, {{512, 0}, {256, 200}}),
      },
    .mGameLayerCount = 2,
  });
  const auto cpp = ExpectParity(script);
  ASSERT_EQ(cpp.size(), 1);
  EXPECT_EQ(cpp[0].mLayerCount, 4);
  EXPECT_EQ(cpp[0].mQuads.size(), 2);
}

TEST(RustLayerParity, GlobalInputLayerNotLast) {
  SKIP_IF_UNSUPPORTED();
  HarnessScript script;
  auto config = MakeConfig(script);
  config.mGlobalInputLayerID = 1;
  script.mSteps.push_back({
    .mConfig = config,
    .mLayers =
      {
        AheadLayer(1, -1.5f, {{0, 0}, {300, 400}}),
        AheadLayer(2, 3, {{300, 0}, {500, 250}}),
        AheadLayer(3, 1.5f, {{0, 400}, {600, 600}}),
      },
    .mHmdPose = Hmd(0, 1.7f, 0, 180),
  });
  ExpectParity(script);
}

TEST(RustLayerParity, NonVrLayerFirst) {
  SKIP_IF_UNSUPPORTED();
  HarnessScript script;
  auto nonVr = MakeLayer(1, {{0, 0}, {200, 100}});
  nonVr.mVREnabled = false;
  script.mSteps.push_back({
    .mConfig = MakeConfig(script),
    .mLayers = {nonVr, MakeLayer(2, {{200, 0}, {600, 500}})},
  });
  const auto cpp = ExpectParity(script);
  ASSERT_EQ(cpp.size(), 1);
  ASSERT_EQ(cpp[0].mQuads.size(), 1);
  // The quirk: the VR layer gets the atlas rect sized for the non-VR layer.
  EXPECT_EQ(cpp[0].mQuads[0].mImageRect.extent.width, 200);
}

TEST(RustLayerParity, GazeZoomAndOpacity) {
  SKIP_IF_UNSUPPORTED();
  HarnessScript script;
  const auto config = MakeConfig(script);// gaze input focus on by default
  const std::vector layers {
    AheadLayer(1, -1.5f, {{0, 0}, {300, 400}}),
    AheadLayer(2, 0, {{300, 0}, {500, 250}}),
  };
  script.mSteps = {
    {.mConfig = config, .mLayers = layers, .mHmdPose = Hmd(0, 1.7f, 0)},
    {.mConfig = config, .mLayers = layers, .mHmdPose = Hmd(0, 1.7f, 0, 90)},
  };
  const auto cpp = ExpectParity(script);
  ASSERT_EQ(cpp.size(), 2);
  EXPECT_EQ(cpp[0].mActiveInGameViewID, 2);
  EXPECT_EQ(cpp[1].mActiveInGameViewID, 0);
}

TEST(RustLayerParity, RecenterMidScript) {
  SKIP_IF_UNSUPPORTED();
  HarnessScript script;
  auto config = MakeConfig(script);
  const std::vector layers {MakeLayer(1, {{0, 0}, {512, 512}})};
  script.mSteps.push_back(
    {.mConfig = config, .mLayers = layers, .mHmdPose = Hmd(0, 1.7f, 0)});
  script.mSteps.push_back(
    {.mConfig = config, .mLayers = layers, .mHmdPose = Hmd(1, 1.6f, 2, 30)});
  config.mVR.mRecenterCount = 1;
  script.mSteps.push_back(
    {.mConfig = config, .mLayers = layers, .mHmdPose = Hmd(1, 1.6f, 2, 30)});
  const auto cpp = ExpectParity(script);
  ASSERT_EQ(cpp.size(), 3);
  ASSERT_EQ(cpp[1].mQuads.size(), 1);
  ASSERT_EQ(cpp[2].mQuads.size(), 1);
  EXPECT_NE(
    cpp[1].mQuads[0].mPose.position.x, cpp[2].mQuads[0].mPose.position.x);
}

TEST(RustLayerParity, NoFeederPassThrough) {
  SKIP_IF_UNSUPPORTED();
  HarnessScript script;
  const HarnessStep attached {
    .mConfig = MakeConfig(script),
    .mLayers = {MakeLayer(1, {{0, 0}, {512, 512}})},
  };
  // Re-attaching starts a new SHM session.
  script.mSteps = {attached, {.mDetach = true}, attached};
  const auto cpp = ExpectParity(script);
  ASSERT_EQ(cpp.size(), 3);
  EXPECT_TRUE(cpp[1].mQuads.empty());
  EXPECT_EQ(cpp[2].mQuads.size(), 1);
}

TEST(RustLayerParity, ZeroGameLayersPassThrough) {
  SKIP_IF_UNSUPPORTED();
  HarnessScript script;
  script.mSteps.push_back({
    .mConfig = MakeConfig(script),
    .mLayers = {MakeLayer(1, {{0, 0}, {512, 512}})},
    .mGameLayerCount = 0,
  });
  const auto cpp = ExpectParity(script);
  ASSERT_EQ(cpp.size(), 1);
  EXPECT_EQ(cpp[0].mLayerCount, 0);
}

TEST(RustLayerParity, AtlasResize) {
  SKIP_IF_UNSUPPORTED();
  HarnessScript script;
  auto config = MakeConfig(script);
  std::vector layers {MakeLayer(1, {{0, 0}, {300, 300}})};
  script.mSteps.push_back({.mConfig = config, .mLayers = layers});
  script.mSteps.push_back({.mConfig = config, .mLayers = layers});
  // The atlas is keyed by the config's texture size only: with an unchanged
  // key, a new layer indexes past the cached atlas (C++ `.at()` terminates),
  // so the feeder's texture size changes along with the layer count.
  layers.push_back(MakeLayer(2, {{300, 0}, {400, 200}}));
  config.mTextureSize = {1024, 1000};
  script.mSteps.push_back({.mConfig = config, .mLayers = layers});
  // A new source size with the same key keeps the cached atlas.
  layers[1].mVR.mLocationOnTexture.mSize = {600, 500};
  script.mSteps.push_back({.mConfig = config, .mLayers = layers});
  // A new key rebuilds the atlas and the swapchain.
  config.mTextureSize = {1000, 1024};
  script.mSteps.push_back({.mConfig = config, .mLayers = layers});
  const auto cpp = ExpectParity(script);
  ASSERT_EQ(cpp.size(), 5);
  ASSERT_EQ(cpp[4].mQuads.size(), 2);
  EXPECT_EQ(cpp[0].mQuads[0].mSwapchainOrdinal, 0);
  EXPECT_EQ(cpp[2].mQuads[0].mSwapchainOrdinal, 1);
  EXPECT_EQ(cpp[3].mQuads[1].mImageRect.extent.width, 400);
  EXPECT_EQ(cpp[4].mQuads[0].mSwapchainOrdinal, 2);
}

TEST(RustLayerParity, VarjoUpscaling) {
  SKIP_IF_UNSUPPORTED();
  HarnessScript script {
    .mRuntimeName = "Varjo OpenXR",
    .mSourceTextureSize = {3000, 1500},
  };
  script.mSteps.push_back({
    .mConfig = MakeConfig(script),
    .mLayers = {MakeLayer(1, {{0, 0}, {3000, 1500}})},
  });
  const auto cpp = ExpectParity(script);
  ASSERT_EQ(cpp.size(), 1);
  ASSERT_EQ(cpp[0].mQuads.size(), 1);
  EXPECT_EQ(cpp[0].mQuads[0].mImageRect.extent.width, 2048);
}

TEST(RustLayerParity, FormatFallback) {
  SKIP_IF_UNSUPPORTED();
  HarnessScript script {
    .mSwapchainFormats = {DXGI_FORMAT_R8G8B8A8_UNORM_SRGB},
  };
  script.mSteps.push_back({
    .mConfig = MakeConfig(script),
    .mLayers = {MakeLayer(1, {{0, 0}, {512, 512}})},
  });
  const auto cpp = ExpectParity(script);
  ASSERT_EQ(cpp.size(), 1);
  EXPECT_EQ(cpp[0].mSwapchainFormat, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB);
}

TEST(RustLayerParity, SameDisplayTimeUsesCachedPose) {
  SKIP_IF_UNSUPPORTED();
  HarnessScript script;
  const auto config = MakeConfig(script);
  const std::vector layers {AheadLayer(1, 0, {{0, 0}, {500, 250}})};
  script.mSteps = {
    {
      .mConfig = config,
      .mLayers = layers,
      .mHmdPose = Hmd(0, 1.7f, 0),
      .mDisplayTime = 5,
    },
    {
      .mConfig = config,
      .mLayers = layers,
      .mHmdPose = Hmd(0, 1.7f, 0, 90),
      .mDisplayTime = 5,
    },
  };
  const auto cpp = ExpectParity(script);
  ASSERT_EQ(cpp.size(), 2);
  // Still looking at the layer: the second HMD pose is never located.
  EXPECT_EQ(cpp[0].mActiveInGameViewID, 1);
  EXPECT_EQ(cpp[1].mActiveInGameViewID, 1);
}
