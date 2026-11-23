// OpenKneeboard
//
// Copyright (c) 2026 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.

// Differential tests: the C++ SpriteAtlas and Geometry2D::ScaledToFit against
// the Rust port in okb-vr (through okb-ffi).
#include <OpenKneeboard/SpriteAtlas.hpp>

#include <gtest/gtest.h>

#include <cstdint>
#include <optional>
#include <random>
#include <span>
#include <stdexcept>
#include <vector>

#include <okb_ffi.h>

using namespace OpenKneeboard;

static_assert(sizeof(OkbPixelSize) == sizeof(PixelSize));
static_assert(sizeof(OkbPixelRect) == sizeof(PixelRect));

namespace {

void ExpectParity(std::vector<PixelSize> sprites) {
  std::vector<OkbPixelSize> in;
  for (const auto& it: sprites) {
    in.push_back({it.mWidth, it.mHeight});
  }
  OkbPixelSize rustSize {};
  std::vector<OkbPixelRect> rustRects(sprites.size());
  const auto rustResult = okb_sprite_atlas_new(
    in.data(), in.size(), &rustSize, rustRects.data());

  std::optional<SpriteAtlas> cpp;
  try {
    cpp.emplace(std::span<PixelSize> {sprites});
  } catch (const std::runtime_error&) {
  }

  ASSERT_EQ(rustResult == 0, cpp.has_value());
  if (!cpp) {
    EXPECT_EQ(rustResult, -1);
    return;
  }
  EXPECT_EQ(rustSize.width, cpp->GetAtlasSize().mWidth);
  EXPECT_EQ(rustSize.height, cpp->GetAtlasSize().mHeight);
  for (size_t i = 0; i < sprites.size(); ++i) {
    const auto expected = cpp->GetRect(static_cast<uint8_t>(i));
    EXPECT_EQ(rustRects[i].offset.x, expected.mOffset.mX) << "sprite " << i;
    EXPECT_EQ(rustRects[i].offset.y, expected.mOffset.mY) << "sprite " << i;
    EXPECT_EQ(rustRects[i].size.width, expected.mSize.mWidth) << "sprite " << i;
    EXPECT_EQ(rustRects[i].size.height, expected.mSize.mHeight)
      << "sprite " << i;
  }
}

std::vector<std::vector<PixelSize>> Inputs() {
  std::vector<std::vector<PixelSize>> ret;
  std::mt19937 rng {12345};
  for (int i = 0; i < 200; ++i) {
    const auto count = std::uniform_int_distribution<int> {1, 16}(rng);
    std::vector<PixelSize> sprites;
    for (int j = 0; j < count; ++j) {
      sprites.push_back({
        std::uniform_int_distribution<uint32_t> {1, 4096}(rng),
        std::uniform_int_distribution<uint32_t> {1, 4096}(rng),
      });
    }
    ret.push_back(std::move(sprites));
  }
  ret.push_back(std::vector<PixelSize>(8, {300, 200}));
  {
    std::vector<PixelSize> huge {{4096, 4096}};
    huge.insert(huge.end(), 15, PixelSize {3, 3});
    ret.push_back(huge);
  }
  ret.push_back(std::vector<PixelSize>(16, {2048, 2048}));
  // Growth by 256, then the resource limit
  ret.push_back({{1030, 1030}, {1030, 1030}});
  ret.push_back({{17000, 17000}, {17000, 17000}});
  ret.push_back({{16384, 16384}});
  return ret;
}

}// namespace

TEST(SpriteAtlasParity, Atlas) {
  for (const auto& sprites: Inputs()) {
    ExpectParity(sprites);
  }
}

TEST(SpriteAtlasParity, ScaledToFit) {
  for (const auto& sprites: Inputs()) {
    for (const auto& size: sprites) {
      const auto expected = size.ScaledToFit(MaxViewRenderSize);
      const auto actual = okb_scaled_to_fit(
        {size.mWidth, size.mHeight},
        {MaxViewRenderSize.mWidth, MaxViewRenderSize.mHeight});
      EXPECT_EQ(actual.width, expected.mWidth);
      EXPECT_EQ(actual.height, expected.mHeight);
    }
  }
}
