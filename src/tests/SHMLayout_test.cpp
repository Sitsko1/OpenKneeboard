// OpenKneeboard
//
// Copyright (c) 2026 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.
#include <OpenKneeboard/SHM.hpp>
#include <OpenKneeboard/SHM/ActiveConsumers.hpp>
#include <OpenKneeboard/SHM/Wire.hpp>
#include <OpenKneeboard/VRSettings.hpp>
#include <OpenKneeboard/json.hpp>

#include <gtest/gtest.h>

#include <cstddef>
#include <cstdlib>
#include <filesystem>
#include <fstream>

using namespace OpenKneeboard;
using json = nlohmann::json;

namespace {

template <class T>
json Entry(json fields) {
  return {{"size", sizeof(T)}, {"align", alignof(T)}, {"fields", fields}};
}

#define F(Type, Member) {#Member, offsetof(Type, Member)}

json ActualLayout() {
  using SHM::Wire::SharedData;
  using Quirks = VRRenderSettings::Quirks;
  using SHM::ActiveConsumers;

  json structs;
  structs["SharedData"] = Entry<SharedData>({
    F(SharedData, mMagic),
    F(SharedData, mGPULUID),
    F(SharedData, mFrameNumber),
    F(SharedData, mSessionID),
    F(SharedData, mFlags),
    F(SharedData, mFeederProcessID),
    F(SharedData, mPadding),
    F(SharedData, mFrames),
  });
  structs["Frame"] = Entry<SharedData::Frame>({
    F(SharedData::Frame, mTexture),
    F(SharedData::Frame, mFence),
    F(SharedData::Frame, mReadyReadFenceValue),
    F(SharedData::Frame, mConfig),
    F(SharedData::Frame, mLayers),
    F(SharedData::Frame, mLayerCount),
  });
  structs["Config"] = Entry<SHM::Config>({
    F(SHM::Config, mGlobalInputLayerID),
    F(SHM::Config, mVR),
    F(SHM::Config, mTextureSize),
    F(SHM::Config, mTint),
  });
  structs["LayerConfig"] = Entry<SHM::LayerConfig>({
    F(SHM::LayerConfig, mLayerID),
    F(SHM::LayerConfig, mVREnabled),
    F(SHM::LayerConfig, mVR),
  });
  structs["VRRenderSettings"] = Entry<VRRenderSettings>({
    F(VRRenderSettings, mQuirks),
    F(VRRenderSettings, mEnableGazeInputFocus),
    F(VRRenderSettings, mForceZoom),
    F(VRRenderSettings, mRecenterCount),
  });
  structs["Quirks"] = Entry<Quirks>({F(Quirks, mOpenXR_Upscaling)});
  structs["VRLayer"] = Entry<SHM::VRLayer>({
    F(SHM::VRLayer, mPose),
    F(SHM::VRLayer, mPhysicalSize),
    F(SHM::VRLayer, mEnableGazeZoom),
    F(SHM::VRLayer, mZoomScale),
    F(SHM::VRLayer, mGazeTargetScale),
    F(SHM::VRLayer, mOpacity),
    F(SHM::VRLayer, mLocationOnTexture),
  });
  structs["VRPose"] = Entry<VRPose>({
    F(VRPose, mX),
    F(VRPose, mEyeY),
    F(VRPose, mZ),
    F(VRPose, mRX),
    F(VRPose, mRY),
    F(VRPose, mRZ),
  });
  structs["GazeTargetScale"] = Entry<GazeTargetScale>({
    F(GazeTargetScale, mVertical),
    F(GazeTargetScale, mHorizontal),
  });
  structs["VROpacitySettings"] = Entry<VROpacitySettings>({
    F(VROpacitySettings, mNormal),
    F(VROpacitySettings, mGaze),
  });
  structs["PixelRect"] = Entry<PixelRect>({
    F(PixelRect, mOffset),
    F(PixelRect, mSize),
    F(PixelRect, mOrigin),
  });
  structs["PixelSize"] = Entry<PixelSize>({
    F(PixelSize, mWidth),
    F(PixelSize, mHeight),
  });
  structs["ActiveConsumers"] = Entry<ActiveConsumers>({
    F(ActiveConsumers, mOpenVR),
    F(ActiveConsumers, mOpenXR_D3D11),
    F(ActiveConsumers, mOpenXR_D3D12),
    F(ActiveConsumers, mOpenXR_Vulkan2),
    F(ActiveConsumers, mViewer),
    F(ActiveConsumers, mElevatedConsumerProcessID),
    F(ActiveConsumers, mNonVRPixelSize),
    F(ActiveConsumers, mGlobalInputViewID),
    F(ActiveConsumers, mActiveViewID),
  });
  return {{"structs", structs}};
}

#undef F

}// namespace

TEST(SHMLayout, MatchesManifest) {
  const auto path =
    std::filesystem::path {OKB_RUST_DIR} / "crates/okb-shm/layout/shm-v1.json";
  const auto actual = ActualLayout();

  if (const auto* update = std::getenv("OKB_UPDATE_SHM_LAYOUT");
      update && std::string_view {update} == "1") {
    std::filesystem::create_directories(path.parent_path());
    std::ofstream(path, std::ios::binary) << actual.dump(2) << '\n';
    GTEST_SKIP() << "Wrote " << path;
  }

  std::ifstream f(path, std::ios::binary);
  ASSERT_TRUE(f.is_open()) << path;
  const auto expected = json::parse(f);
  EXPECT_EQ(expected, actual) << "Actual layout:\n" << actual.dump(2);
}
