// OpenKneeboard
//
// Copyright (c) 2026 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.
#pragma once

// Loads an OpenXR API layer DLL against a fake OpenXR runtime and an
// in-process D3D11 SHM feeder, and records what the layer submits to the
// runtime on every xrEndFrame.

#include <OpenKneeboard/Pixels.hpp>
#include <OpenKneeboard/SHM.hpp>

#include <openxr/openxr.h>

#include <cstdint>
#include <filesystem>
#include <string>
#include <vector>

#include <dxgiformat.h>

namespace OpenKneeboard::Tests {

struct HarnessStep {
  SHM::Config mConfig;
  std::vector<SHM::LayerConfig> mLayers;// empty + mDetach = writer detached
  bool mDetach {false};
  XrPosef mHmdPose {.orientation = {0, 0, 0, 1}};
  // Non-zero: the layer caches the HMD pose by display time, starting at 0.
  XrTime mDisplayTime {1};
  uint32_t mGameLayerCount {1};
};

struct HarnessScript {
  std::string mRuntimeName {"FakeRuntime"};
  uint32_t mMaxLayerCount {16};
  std::vector<int64_t> mSwapchainFormats {
    DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
    DXGI_FORMAT_R8G8B8A8_UNORM_SRGB};
  PixelSize mSourceTextureSize {1024, 1024};
  std::vector<HarnessStep> mSteps;
};

struct RecordedQuad {
  XrPosef mPose;
  XrExtent2Df mSize;
  int mSwapchainOrdinal;// creation order; -1 if unknown
  XrRect2Di mImageRect;
  XrCompositionLayerFlags mFlags;
  XrEyeVisibility mEye;
  std::string mSpace;// "LOCAL"/"VIEW"/"OTHER"
};

struct RecordedFrame {
  bool mGameLayersForwardedUnchanged;
  uint32_t mLayerCount;
  std::vector<RecordedQuad> mQuads;
  // The image released this frame; BGRA/RGBA as created, tightly packed.
  std::vector<uint8_t> mSwapchainPixels;
  uint32_t mSwapchainWidth {};// 0 if no image was released this frame
  uint64_t mActiveInGameViewID;
};

// False if no D3D11 adapter (hardware, then WARP) can create and reopen a
// shared NT-handle BGRA8 texture and a shared fence; callers skip with
// HarnessSkipMessage.
bool HarnessSupported();
inline constexpr char HarnessSkipMessage[] =
  "no D3D11 adapter supports shared NT-handle textures and fences";
// e.g. "WARP: Microsoft Basic Render Driver"; empty if unsupported.
std::string HarnessAdapterDescription();

std::vector<RecordedFrame> RunHarness(
  const std::filesystem::path& layerDll,
  const HarnessScript&);

}// namespace OpenKneeboard::Tests
