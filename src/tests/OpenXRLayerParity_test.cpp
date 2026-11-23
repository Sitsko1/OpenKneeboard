// OpenKneeboard
//
// Copyright (c) 2026 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.

// Characterizes the C++ OpenXR layer through the harness in
// OpenXRLayerHarness.hpp.

#include "OpenXRLayerHarness.hpp"

#include <OpenKneeboard/SpriteAtlas.hpp>

#include <gtest/gtest.h>

#include <algorithm>

using namespace OpenKneeboard;
using namespace OpenKneeboard::Tests;

namespace {

const std::filesystem::path CppLayer {OKB_TEST_CPP_LAYER_DLL};

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
