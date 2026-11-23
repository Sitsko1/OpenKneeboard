// OpenKneeboard
//
// Copyright (c) 2026 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.

// Loads each built OpenXR layer DLL (C++ and Rust) the way the OpenXR loader
// does, and checks that it negotiates only under the configured layer name.

#include <OpenKneeboard/config.hpp>

#include <Windows.h>

#include <openxr/openxr.h>

#include <gtest/gtest.h>

#include <loader_interfaces.h>

#include <string>

namespace {

using NegotiateFn = XrResult(XRAPI_PTR*)(
  const XrNegotiateLoaderInfo*,
  const char*,
  XrNegotiateApiLayerRequest*);

class OpenXRLayerLoad : public ::testing::TestWithParam<const wchar_t*> {
 protected:
  void SetUp() override {
    mModule = LoadLibraryW(GetParam());
    ASSERT_NE(mModule, nullptr)
      << "LoadLibraryW failed: " << GetLastError();
  }

  void TearDown() override {
    if (mModule) {
      FreeLibrary(mModule);
    }
  }

  NegotiateFn GetNegotiate() const {
    return reinterpret_cast<NegotiateFn>(GetProcAddress(
      mModule, "OpenKneeboard_xrNegotiateLoaderApiLayerInterface"));
  }

  static XrResult Negotiate(NegotiateFn fn, const char* layerName) {
    XrNegotiateLoaderInfo loaderInfo {
      .structType = XR_LOADER_INTERFACE_STRUCT_LOADER_INFO,
      .structVersion = XR_LOADER_INFO_STRUCT_VERSION,
      .structSize = sizeof(XrNegotiateLoaderInfo),
      .minInterfaceVersion = 1,
      .maxInterfaceVersion = XR_CURRENT_LOADER_API_LAYER_VERSION,
      .minApiVersion = XR_MAKE_VERSION(1, 0, 0),
      .maxApiVersion = XR_CURRENT_API_VERSION,
    };
    XrNegotiateApiLayerRequest request {
      .structType = XR_LOADER_INTERFACE_STRUCT_API_LAYER_REQUEST,
      .structVersion = XR_API_LAYER_INFO_STRUCT_VERSION,
      .structSize = sizeof(XrNegotiateApiLayerRequest),
    };
    const auto result = fn(&loaderInfo, layerName, &request);
    if (result == XR_SUCCESS) {
      EXPECT_EQ(
        request.layerInterfaceVersion, XR_CURRENT_LOADER_API_LAYER_VERSION);
      EXPECT_NE(request.getInstanceProcAddr, nullptr);
      EXPECT_NE(request.createApiLayerInstance, nullptr);
    }
    return result;
  }

  HMODULE mModule {nullptr};
};

TEST_P(OpenXRLayerLoad, ExportsNegotiate) {
  EXPECT_NE(GetNegotiate(), nullptr);
}

TEST_P(OpenXRLayerLoad, NegotiatesWithLayerName) {
  const auto fn = GetNegotiate();
  ASSERT_NE(fn, nullptr);
  const std::string name {OpenKneeboard::OpenXRApiLayerName};
  EXPECT_EQ(Negotiate(fn, name.c_str()), XR_SUCCESS);
}

TEST_P(OpenXRLayerLoad, RejectsWrongLayerName) {
  const auto fn = GetNegotiate();
  ASSERT_NE(fn, nullptr);
  EXPECT_EQ(
    Negotiate(fn, "XR_APILAYER_wrong"), XR_ERROR_INITIALIZATION_FAILED);
}

INSTANTIATE_TEST_SUITE_P(
  Layers,
  OpenXRLayerLoad,
  ::testing::Values(
    L"" OKB_TEST_CPP_LAYER_DLL,
    L"" OKB_TEST_RUST_LAYER_DLL),
  [](const auto& info) { return info.index == 0 ? "Cpp" : "Rust"; });

}// namespace
