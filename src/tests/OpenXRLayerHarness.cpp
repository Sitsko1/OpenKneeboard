// OpenKneeboard
//
// Copyright (c) 2026 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.

#include "OpenXRLayerHarness.hpp"

#include <OpenKneeboard/SHM/ActiveConsumers.hpp>

#include <OpenKneeboard/config.hpp>
#include <OpenKneeboard/scope_exit.hpp>

#include <shims/winrt/base.h>

#include <Windows.h>

#include <d3d11_4.h>
#include <dxgi1_2.h>

#define XR_USE_GRAPHICS_API_D3D11
#include <openxr/openxr_platform.h>

#include <algorithm>
#include <bit>
#include <cstring>
#include <format>
#include <map>
#include <memory>
#include <mutex>
#include <optional>
#include <stdexcept>
#include <string_view>
#include <utility>

#include <loader_interfaces.h>

namespace OpenKneeboard::Tests {

namespace {

void Check(bool ok, std::string_view what) {
  if (!ok) {
    throw std::runtime_error(std::format("OpenXR layer harness: {}", what));
  }
}

void CheckHR(HRESULT hr, std::string_view what) {
  Check(
    SUCCEEDED(hr),
    std::format("{} failed: {:#010x}", what, static_cast<uint32_t>(hr)));
}

void CheckXR(XrResult result, std::string_view what) {
  Check(
    XR_SUCCEEDED(result),
    std::format("{} returned {}", what, static_cast<int>(result)));
}

// XR handles are pointers on 64-bit and uint64_t on 32-bit.
template <class T>
T ToHandle(uint64_t value) {
  return std::bit_cast<T>(value);
}
template <class T>
uint64_t FromHandle(T handle) {
  return std::bit_cast<uint64_t>(handle);
}

///// D3D11 /////

struct Device {
  winrt::com_ptr<ID3D11Device5> mDevice;
  winrt::com_ptr<ID3D11DeviceContext4> mContext;
  uint64_t mLuid {};
  std::string mDescription;
};

std::optional<Device> CreateDevice(D3D_DRIVER_TYPE type) {
  winrt::com_ptr<ID3D11Device> device;
  winrt::com_ptr<ID3D11DeviceContext> context;
  D3D_FEATURE_LEVEL level = D3D_FEATURE_LEVEL_11_1;
  if (
    FAILED(D3D11CreateDevice(
      nullptr,
      type,
      nullptr,
      D3D11_CREATE_DEVICE_BGRA_SUPPORT,
      &level,
      1,
      D3D11_SDK_VERSION,
      device.put(),
      nullptr,
      context.put()))) {
    return std::nullopt;
  }
  Device ret {
    .mDevice = device.try_as<ID3D11Device5>(),
    .mContext = context.try_as<ID3D11DeviceContext4>(),
  };
  if (!(ret.mDevice && ret.mContext)) {
    return std::nullopt;
  }
  winrt::com_ptr<IDXGIAdapter> adapter;
  CheckHR(
    device.as<IDXGIDevice>()->GetAdapter(adapter.put()),
    "IDXGIDevice::GetAdapter");
  DXGI_ADAPTER_DESC desc {};
  CheckHR(adapter->GetDesc(&desc), "IDXGIAdapter::GetDesc");
  ret.mLuid = std::bit_cast<uint64_t>(desc.AdapterLuid);
  std::string name;
  for (const auto c: std::wstring_view {desc.Description}) {
    name += (c < 0x80) ? static_cast<char>(c) : '?';
  }
  ret.mDescription = std::format(
    "{}: {}", type == D3D_DRIVER_TYPE_WARP ? "WARP" : "hardware", name);
  return ret;
}

struct SharedResources {
  winrt::com_ptr<ID3D11Texture2D> mTexture;
  winrt::handle mTextureHandle;
  winrt::com_ptr<ID3D11Fence> mFence;
  winrt::handle mFenceHandle;
};

std::optional<SharedResources> CreateShared(
  const Device& device,
  const PixelSize& size,
  const std::vector<uint8_t>* initialData) {
  const D3D11_TEXTURE2D_DESC desc {
    .Width = size.mWidth,
    .Height = size.mHeight,
    .MipLevels = 1,
    .ArraySize = 1,
    .Format = SHM::SHARED_TEXTURE_PIXEL_FORMAT,
    .SampleDesc = {1, 0},
    .BindFlags = D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET,
    .MiscFlags =
      D3D11_RESOURCE_MISC_SHARED | D3D11_RESOURCE_MISC_SHARED_NTHANDLE,
  };
  D3D11_SUBRESOURCE_DATA data {};
  if (initialData) {
    data = {initialData->data(), size.mWidth * 4, 0};
  }
  SharedResources ret;
  if (
    FAILED(device.mDevice->CreateTexture2D(
      &desc, initialData ? &data : nullptr, ret.mTexture.put()))) {
    return std::nullopt;
  }
  if (
    FAILED(ret.mTexture.as<IDXGIResource1>()->CreateSharedHandle(
      nullptr, DXGI_SHARED_RESOURCE_READ, nullptr, ret.mTextureHandle.put()))) {
    return std::nullopt;
  }
  if (
    FAILED(device.mDevice->CreateFence(
      0, D3D11_FENCE_FLAG_SHARED, IID_PPV_ARGS(ret.mFence.put())))) {
    return std::nullopt;
  }
  if (
    FAILED(ret.mFence->CreateSharedHandle(
      nullptr, GENERIC_ALL, nullptr, ret.mFenceHandle.put()))) {
    return std::nullopt;
  }
  return ret;
}

bool Probe(const Device& device) {
  const auto shared = CreateShared(device, {64, 64}, nullptr);
  if (!shared) {
    return false;
  }
  winrt::com_ptr<ID3D11Texture2D> texture;
  winrt::com_ptr<ID3D11Fence> fence;
  return SUCCEEDED(device.mDevice->OpenSharedResource1(
           shared->mTextureHandle.get(), IID_PPV_ARGS(texture.put())))
    && SUCCEEDED(device.mDevice->OpenSharedFence(
      shared->mFenceHandle.get(), IID_PPV_ARGS(fence.put())));
}

// Hardware adapter, else WARP; nullopt if neither passes the probe.
const std::optional<std::pair<D3D_DRIVER_TYPE, std::string>>& ProbeResult() {
  static const auto sResult =
    []() -> std::optional<std::pair<D3D_DRIVER_TYPE, std::string>> {
    for (const auto type: {D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP}) {
      const auto device = CreateDevice(type);
      if (device && Probe(*device)) {
        return std::pair {type, device->mDescription};
      }
    }
    return std::nullopt;
  }();
  return sResult;
}

// texel (x, y) = premultiplied (x % 251, y % 241, (x + y) % 239, 255), as BGRA
std::vector<uint8_t> MakePattern(const PixelSize& size) {
  std::vector<uint8_t> ret(size.mWidth * size.mHeight * 4);
  for (uint32_t y = 0; y < size.mHeight; ++y) {
    for (uint32_t x = 0; x < size.mWidth; ++x) {
      const auto p = &ret[(y * size.mWidth + x) * 4];
      p[0] = static_cast<uint8_t>((x + y) % 239);
      p[1] = static_cast<uint8_t>(y % 241);
      p[2] = static_cast<uint8_t>(x % 251);
      p[3] = 255;
    }
  }
  return ret;
}

DXGI_FORMAT TypelessFormat(int64_t format) {
  switch (format) {
    case DXGI_FORMAT_B8G8R8A8_UNORM:
    case DXGI_FORMAT_B8G8R8A8_UNORM_SRGB:
      return DXGI_FORMAT_B8G8R8A8_TYPELESS;
    case DXGI_FORMAT_R8G8B8A8_UNORM:
    case DXGI_FORMAT_R8G8B8A8_UNORM_SRGB:
      return DXGI_FORMAT_R8G8B8A8_TYPELESS;
    default:
      return static_cast<DXGI_FORMAT>(format);
  }
}

///// Fake runtime /////

constexpr uint64_t InstanceHandle = 0x1001;
constexpr uint64_t SessionHandle = 0x2001;
constexpr XrSystemId SystemId = 0x3001;
constexpr uint32_t SwapchainImageCount = 3;

struct FakeSwapchain {
  int mOrdinal {};
  int64_t mFormat {};
  uint32_t mWidth {};
  uint32_t mHeight {};
  std::vector<winrt::com_ptr<ID3D11Texture2D>> mImages;
  winrt::com_ptr<ID3D11Texture2D> mStaging;
  uint32_t mNextImage {};
  uint32_t mAcquired {};
};

struct State {
  const HarnessScript& mScript;
  const Device& mDevice;
  const HarnessStep* mStep {};
  const XrFrameEndInfo* mGameEndInfo {};
  std::vector<const XrCompositionLayerBaseHeader*> mGameLayers;
  std::map<uint64_t, XrReferenceSpaceType> mSpaces;
  std::map<uint64_t, FakeSwapchain> mSwapchains;
  uint64_t mNextHandle {0x10000};
  std::vector<uint8_t> mReleasedPixels;
  uint32_t mReleasedWidth {};
  int64_t mReleasedFormat {};
  std::vector<RecordedFrame> mFrames;
  // The fakes never throw: a C++ exception must not unwind through the layer's
  // frames (UB for the Rust layer). They record the first error here and
  // return an XrResult; the driver rethrows it once the layer call returns.
  std::string mError;
};

// ponytail: one run at a time; the layer under test keeps global state too.
State* gState {nullptr};

namespace Fake {

XrResult Fail(std::string_view what) {
  if (gState->mError.empty()) {
    gState->mError = std::format("OpenXR layer harness: fake runtime: {}", what);
  }
  return XR_ERROR_RUNTIME_FAILURE;
}

FakeSwapchain* FindSwapchain(XrSwapchain swapchain) {
  const auto it = gState->mSwapchains.find(FromHandle(swapchain));
  return it == gState->mSwapchains.end() ? nullptr : &it->second;
}

XRAPI_ATTR XrResult XRAPI_CALL
xrGetInstanceProcAddr(XrInstance, const char* name, PFN_xrVoidFunction*);

XRAPI_ATTR XrResult XRAPI_CALL createApiLayerInstance(
  const XrInstanceCreateInfo*,
  const XrApiLayerCreateInfo*,
  XrInstance* instance) {
  *instance = ToHandle<XrInstance>(InstanceHandle);
  return XR_SUCCESS;
}

XRAPI_ATTR XrResult XRAPI_CALL
xrGetInstanceProperties(XrInstance, XrInstanceProperties* properties) {
  properties->runtimeVersion = XR_MAKE_VERSION(1, 0, 0);
  strncpy_s(
    properties->runtimeName, gState->mScript.mRuntimeName.c_str(), _TRUNCATE);
  return XR_SUCCESS;
}

XRAPI_ATTR XrResult XRAPI_CALL
xrGetSystemProperties(XrInstance, XrSystemId, XrSystemProperties* properties) {
  properties->systemId = SystemId;
  strncpy_s(properties->systemName, "FakeSystem", _TRUNCATE);
  properties->graphicsProperties = {
    .maxSwapchainImageHeight = 4096,
    .maxSwapchainImageWidth = 4096,
    .maxLayerCount = gState->mScript.mMaxLayerCount,
  };
  return XR_SUCCESS;
}

XRAPI_ATTR XrResult XRAPI_CALL
xrCreateSession(XrInstance, const XrSessionCreateInfo*, XrSession* session) {
  *session = ToHandle<XrSession>(SessionHandle);
  return XR_SUCCESS;
}

XRAPI_ATTR XrResult XRAPI_CALL xrDestroySession(XrSession) {
  return XR_SUCCESS;
}

XRAPI_ATTR XrResult XRAPI_CALL xrDestroyInstance(XrInstance) {
  return XR_SUCCESS;
}

XRAPI_ATTR XrResult XRAPI_CALL xrCreateReferenceSpace(
  XrSession,
  const XrReferenceSpaceCreateInfo* info,
  XrSpace* space) {
  const auto handle = gState->mNextHandle++;
  gState->mSpaces[handle] = info->referenceSpaceType;
  *space = ToHandle<XrSpace>(handle);
  return XR_SUCCESS;
}

XRAPI_ATTR XrResult XRAPI_CALL xrDestroySpace(XrSpace space) {
  return gState->mSpaces.erase(FromHandle(space)) ? XR_SUCCESS
                                                  : XR_ERROR_HANDLE_INVALID;
}

XRAPI_ATTR XrResult XRAPI_CALL
xrLocateSpace(XrSpace, XrSpace, XrTime, XrSpaceLocation* location) {
  if (!gState->mStep) {
    return Fail("xrLocateSpace outside a step");
  }
  location->locationFlags = XR_SPACE_LOCATION_ORIENTATION_VALID_BIT
    | XR_SPACE_LOCATION_POSITION_VALID_BIT;
  location->pose = gState->mStep->mHmdPose;
  return XR_SUCCESS;
}

XRAPI_ATTR XrResult XRAPI_CALL xrEnumerateInstanceExtensionProperties(
  const char*,
  uint32_t,
  uint32_t* count,
  XrExtensionProperties*) {
  *count = 0;
  return XR_SUCCESS;
}

XRAPI_ATTR XrResult XRAPI_CALL xrEnumerateSwapchainFormats(
  XrSession,
  uint32_t capacity,
  uint32_t* count,
  int64_t* formats) {
  const auto& available = gState->mScript.mSwapchainFormats;
  *count = static_cast<uint32_t>(available.size());
  if (capacity == 0) {
    return XR_SUCCESS;
  }
  if (capacity < available.size()) {
    return XR_ERROR_SIZE_INSUFFICIENT;
  }
  std::ranges::copy(available, formats);
  return XR_SUCCESS;
}

XRAPI_ATTR XrResult XRAPI_CALL xrCreateSwapchain(
  XrSession,
  const XrSwapchainCreateInfo* info,
  XrSwapchain* swapchain) {
  const auto& device = gState->mDevice;
  FakeSwapchain sc {
    .mOrdinal = static_cast<int>(gState->mSwapchains.size()),
    .mFormat = info->format,
    .mWidth = info->width,
    .mHeight = info->height,
  };
  D3D11_TEXTURE2D_DESC desc {
    .Width = info->width,
    .Height = info->height,
    .MipLevels = 1,
    .ArraySize = 1,
    // As real runtimes: typeless, so the app picks UNORM or SRGB views.
    .Format = TypelessFormat(info->format),
    .SampleDesc = {1, 0},
    .BindFlags = D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE,
  };
  for (uint32_t i = 0; i < SwapchainImageCount; ++i) {
    if (FAILED(device.mDevice->CreateTexture2D(
          &desc, nullptr, sc.mImages.emplace_back().put()))) {
      return Fail("CreateTexture2D(swapchain)");
    }
  }
  desc.Usage = D3D11_USAGE_STAGING;
  desc.BindFlags = 0;
  desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
  if (FAILED(
        device.mDevice->CreateTexture2D(&desc, nullptr, sc.mStaging.put()))) {
    return Fail("CreateTexture2D(staging)");
  }

  const auto handle = gState->mNextHandle++;
  gState->mSwapchains.emplace(handle, std::move(sc));
  *swapchain = ToHandle<XrSwapchain>(handle);
  return XR_SUCCESS;
}

XRAPI_ATTR XrResult XRAPI_CALL xrDestroySwapchain(XrSwapchain swapchain) {
  // Kept: ordinals are creation order, and quads refer to it by handle.
  return gState->mSwapchains.contains(FromHandle(swapchain))
    ? XR_SUCCESS
    : XR_ERROR_HANDLE_INVALID;
}

XRAPI_ATTR XrResult XRAPI_CALL xrEnumerateSwapchainImages(
  XrSwapchain swapchain,
  uint32_t capacity,
  uint32_t* count,
  XrSwapchainImageBaseHeader* images) {
  const auto sc = FindSwapchain(swapchain);
  if (!sc) {
    return Fail("xrEnumerateSwapchainImages: unknown swapchain");
  }
  *count = SwapchainImageCount;
  if (capacity == 0) {
    return XR_SUCCESS;
  }
  if (capacity < SwapchainImageCount) {
    return XR_ERROR_SIZE_INSUFFICIENT;
  }
  const auto d3d = reinterpret_cast<XrSwapchainImageD3D11KHR*>(images);
  for (uint32_t i = 0; i < SwapchainImageCount; ++i) {
    d3d[i].texture = sc->mImages[i].get();
  }
  return XR_SUCCESS;
}

XRAPI_ATTR XrResult XRAPI_CALL xrAcquireSwapchainImage(
  XrSwapchain swapchain,
  const XrSwapchainImageAcquireInfo*,
  uint32_t* index) {
  const auto sc = FindSwapchain(swapchain);
  if (!sc) {
    return Fail("xrAcquireSwapchainImage: unknown swapchain");
  }
  sc->mAcquired = sc->mNextImage;
  sc->mNextImage = (sc->mNextImage + 1) % SwapchainImageCount;
  *index = sc->mAcquired;
  return XR_SUCCESS;
}

XRAPI_ATTR XrResult XRAPI_CALL
xrWaitSwapchainImage(XrSwapchain, const XrSwapchainImageWaitInfo*) {
  return XR_SUCCESS;
}

XRAPI_ATTR XrResult XRAPI_CALL xrReleaseSwapchainImage(
  XrSwapchain swapchain,
  const XrSwapchainImageReleaseInfo*) {
  const auto found = FindSwapchain(swapchain);
  if (!found) {
    return Fail("xrReleaseSwapchainImage: unknown swapchain");
  }
  auto& sc = *found;
  const auto& ctx = gState->mDevice.mContext;
  ctx->CopyResource(sc.mStaging.get(), sc.mImages[sc.mAcquired].get());
  D3D11_MAPPED_SUBRESOURCE mapped {};
  if (FAILED(ctx->Map(sc.mStaging.get(), 0, D3D11_MAP_READ, 0, &mapped))) {
    return Fail("Map(staging)");
  }
  const auto rowBytes = sc.mWidth * 4;
  gState->mReleasedPixels.resize(rowBytes * sc.mHeight);
  for (uint32_t y = 0; y < sc.mHeight; ++y) {
    memcpy(
      gState->mReleasedPixels.data() + y * rowBytes,
      static_cast<const uint8_t*>(mapped.pData) + y * mapped.RowPitch,
      rowBytes);
  }
  ctx->Unmap(sc.mStaging.get(), 0);
  gState->mReleasedWidth = sc.mWidth;
  gState->mReleasedFormat = sc.mFormat;
  return XR_SUCCESS;
}

const char* SpaceName(const XrSpace space) {
  const auto it = gState->mSpaces.find(FromHandle(space));
  if (it == gState->mSpaces.end()) {
    return "OTHER";
  }
  switch (it->second) {
    case XR_REFERENCE_SPACE_TYPE_LOCAL:
      return "LOCAL";
    case XR_REFERENCE_SPACE_TYPE_VIEW:
      return "VIEW";
    default:
      return "OTHER";
  }
}

XRAPI_ATTR XrResult XRAPI_CALL
xrEndFrame(XrSession, const XrFrameEndInfo* info) {
  const auto& game = gState->mGameLayers;
  const auto gameInfo = gState->mGameEndInfo;
  if (!gameInfo) {
    return Fail("xrEndFrame outside a step");
  }
  RecordedFrame frame {
    .mGameLayersForwardedUnchanged = info->type == gameInfo->type
      && info->next == gameInfo->next
      && info->displayTime == gameInfo->displayTime
      && info->environmentBlendMode == gameInfo->environmentBlendMode
      && info->layerCount >= game.size()
      && std::equal(game.begin(), game.end(), info->layers),
    .mLayerCount = info->layerCount,
    .mSwapchainPixels = std::exchange(gState->mReleasedPixels, {}),
    .mSwapchainWidth = std::exchange(gState->mReleasedWidth, 0),
    .mSwapchainFormat = std::exchange(gState->mReleasedFormat, 0),
  };
  for (auto i = static_cast<uint32_t>(game.size()); i < info->layerCount; ++i) {
    if (info->layers[i]->type != XR_TYPE_COMPOSITION_LAYER_QUAD) {
      continue;
    }
    const auto& quad =
      *reinterpret_cast<const XrCompositionLayerQuad*>(info->layers[i]);
    const auto sc = FindSwapchain(quad.subImage.swapchain);
    frame.mQuads.push_back({
      .mPose = quad.pose,
      .mSize = quad.size,
      .mSwapchainOrdinal = sc ? sc->mOrdinal : -1,
      .mImageRect = quad.subImage.imageRect,
      .mImageArrayIndex = quad.subImage.imageArrayIndex,
      .mFlags = quad.layerFlags,
      .mEye = quad.eyeVisibility,
      .mSpace = SpaceName(quad.space),
    });
  }
  gState->mFrames.push_back(std::move(frame));
  return XR_SUCCESS;
}

XRAPI_ATTR XrResult XRAPI_CALL xrGetInstanceProcAddr(
  XrInstance,
  const char* nameCStr,
  PFN_xrVoidFunction* function) {
  const std::string_view name {nameCStr};
#define FAKE_FN(fn) \
  if (name == #fn) { \
    *function = reinterpret_cast<PFN_xrVoidFunction>(&Fake::fn); \
    return XR_SUCCESS; \
  }
  // Every function in OPENKNEEBOARD_NEXT_OPENXR_FUNCS, except the
  // XR_KHR_vulkan_enable2 ones.
  FAKE_FN(xrGetInstanceProcAddr)
  FAKE_FN(xrCreateSession)
  FAKE_FN(xrDestroySession)
  FAKE_FN(xrDestroyInstance)
  FAKE_FN(xrEndFrame)
  FAKE_FN(xrAcquireSwapchainImage)
  FAKE_FN(xrCreateReferenceSpace)
  FAKE_FN(xrCreateSwapchain)
  FAKE_FN(xrDestroySpace)
  FAKE_FN(xrDestroySwapchain)
  FAKE_FN(xrEnumerateInstanceExtensionProperties)
  FAKE_FN(xrEnumerateSwapchainFormats)
  FAKE_FN(xrEnumerateSwapchainImages)
  FAKE_FN(xrGetInstanceProperties)
  FAKE_FN(xrGetSystemProperties)
  FAKE_FN(xrLocateSpace)
  FAKE_FN(xrReleaseSwapchainImage)
  FAKE_FN(xrWaitSwapchainImage)
#undef FAKE_FN
  *function = nullptr;
  return XR_ERROR_FUNCTION_UNSUPPORTED;
}

}// namespace Fake

using NegotiateFn = XrResult(XRAPI_PTR*)(
  const XrNegotiateLoaderInfo*,
  const char*,
  XrNegotiateApiLayerRequest*);

template <class T>
T GetLayerFn(
  PFN_xrGetInstanceProcAddr gipa,
  XrInstance instance,
  const char* name) {
  PFN_xrVoidFunction fn {nullptr};
  CheckXR(gipa(instance, name, &fn), name);
  Check(fn != nullptr, name);
  return reinterpret_cast<T>(fn);
}

}// namespace

bool HarnessSupported() { return ProbeResult().has_value(); }

std::string HarnessAdapterDescription() {
  return ProbeResult() ? ProbeResult()->second : std::string {};
}

std::vector<RecordedFrame> RunHarness(
  const std::filesystem::path& layerDll,
  const HarnessScript& script) {
  Check(HarnessSupported(), HarnessSkipMessage);
  const auto device = CreateDevice(ProbeResult()->first);
  Check(device.has_value(), "D3D11CreateDevice");

  ///// Feeder /////
  const auto pattern = MakePattern(script.mSourceTextureSize);
  const auto shared =
    CreateShared(*device, script.mSourceTextureSize, &pattern);
  Check(shared.has_value(), "creating the shared texture and fence");
  SHM::ActiveConsumers::SetActiveInGameViewID({});
  SHM::Writer writer {device->mLuid};
  Check(static_cast<bool>(writer), "SHM::Writer");

  State state {.mScript = script, .mDevice = *device};
  gState = &state;
  const std::unique_ptr<State*, void (*)(State**)> resetState {
    &gState, [](State** p) { *p = nullptr; }};

  ///// Layer /////
  const std::unique_ptr<std::remove_pointer_t<HMODULE>, decltype(&FreeLibrary)>
    module {LoadLibraryW(layerDll.c_str()), &FreeLibrary};
  Check(module != nullptr, std::format("LoadLibraryW: {}", GetLastError()));
  const auto negotiate = reinterpret_cast<NegotiateFn>(GetProcAddress(
    module.get(), "OpenKneeboard_xrNegotiateLoaderApiLayerInterface"));
  Check(negotiate != nullptr, "negotiate export");

  const std::string layerName {OpenXRApiLayerName};
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
  CheckXR(negotiate(&loaderInfo, layerName.c_str(), &request), "negotiate");

  XrApiLayerNextInfo nextInfo {
    .structType = XR_LOADER_INTERFACE_STRUCT_API_LAYER_NEXT_INFO,
    .structVersion = XR_API_LAYER_NEXT_INFO_STRUCT_VERSION,
    .structSize = sizeof(XrApiLayerNextInfo),
    .nextGetInstanceProcAddr = &Fake::xrGetInstanceProcAddr,
    .nextCreateApiLayerInstance = &Fake::createApiLayerInstance,
  };
  strncpy_s(nextInfo.layerName, layerName.c_str(), _TRUNCATE);
  XrApiLayerCreateInfo layerCreateInfo {
    .structType = XR_LOADER_INTERFACE_STRUCT_API_LAYER_CREATE_INFO,
    .structVersion = XR_API_LAYER_CREATE_INFO_STRUCT_VERSION,
    .structSize = sizeof(XrApiLayerCreateInfo),
    .nextInfo = &nextInfo,
  };
  XrInstanceCreateInfo instanceInfo {.type = XR_TYPE_INSTANCE_CREATE_INFO};
  strncpy_s(
    instanceInfo.applicationInfo.applicationName, "OKBLayerHarness", _TRUNCATE);
  instanceInfo.applicationInfo.apiVersion = XR_CURRENT_API_VERSION;
  // Rethrows a fake-runtime error once the layer call has returned.
  const auto checkLayerCall = [&state](XrResult result, std::string_view what) {
    if (!state.mError.empty()) {
      throw std::runtime_error(std::exchange(state.mError, {}));
    }
    CheckXR(result, what);
  };

  XrInstance instance {};
  checkLayerCall(
    request.createApiLayerInstance(&instanceInfo, &layerCreateInfo, &instance),
    "createApiLayerInstance");

  const auto gipa = request.getInstanceProcAddr;
  const auto layerDestroyInstance =
    GetLayerFn<PFN_xrDestroyInstance>(gipa, instance, "xrDestroyInstance");
  // If the driver throws, still tear the layer down before FreeLibrary.
  const scope_fail destroyInstanceOnThrow {
    [&] { layerDestroyInstance(instance); }};
  const auto layerCreateSession =
    GetLayerFn<PFN_xrCreateSession>(gipa, instance, "xrCreateSession");
  const auto layerEndFrame =
    GetLayerFn<PFN_xrEndFrame>(gipa, instance, "xrEndFrame");
  const auto layerDestroySession =
    GetLayerFn<PFN_xrDestroySession>(gipa, instance, "xrDestroySession");

  XrGraphicsBindingD3D11KHR binding {
    .type = XR_TYPE_GRAPHICS_BINDING_D3D11_KHR,
    .device = device->mDevice.get(),
  };
  XrSessionCreateInfo sessionInfo {
    .type = XR_TYPE_SESSION_CREATE_INFO,
    .next = &binding,
    .systemId = SystemId,
  };
  XrSession session {};
  checkLayerCall(
    layerCreateSession(instance, &sessionInfo, &session), "xrCreateSession");
  const scope_fail destroySessionOnThrow {
    [&] { layerDestroySession(session); }};

  // Opaque to the layer and the fake: only its address is compared.
  const XrBaseInStructure endInfoNext {.type = XR_TYPE_UNKNOWN};

  ///// Steps /////
  for (size_t stepIndex = 0; stepIndex < script.mSteps.size(); ++stepIndex) {
    const auto& step = script.mSteps[stepIndex];
    state.mStep = &step;
    SHM::ActiveConsumers::SetActiveInGameViewID(
      ActiveInGameViewSentinel(stepIndex));
    {
      std::unique_lock lock(writer);
      if (step.mDetach) {
        writer.Detach();
      } else {
        const auto info = writer.BeginFrame();
        CheckHR(
          device->mContext->Signal(shared->mFence.get(), info.mFenceOut),
          "ID3D11DeviceContext4::Signal");
        device->mContext->Flush();
        writer.SubmitFrame(
          info,
          step.mConfig,
          step.mLayers,
          shared->mTextureHandle.get(),
          shared->mFenceHandle.get());
      }
    }

    std::vector<XrCompositionLayerProjection> gameLayers(
      step.mGameLayerCount, {.type = XR_TYPE_COMPOSITION_LAYER_PROJECTION});
    state.mGameLayers.clear();
    for (const auto& layer: gameLayers) {
      state.mGameLayers.push_back(
        reinterpret_cast<const XrCompositionLayerBaseHeader*>(&layer));
    }
    const XrFrameEndInfo endInfo {
      .type = XR_TYPE_FRAME_END_INFO,
      .next = &endInfoNext,
      .displayTime =
        step.mDisplayTime.value_or(static_cast<XrTime>(stepIndex + 1)),
      .environmentBlendMode = XR_ENVIRONMENT_BLEND_MODE_ALPHA_BLEND,
      .layerCount = step.mGameLayerCount,
      .layers = state.mGameLayers.data(),
    };
    state.mGameEndInfo = &endInfo;
    const auto framesBefore = state.mFrames.size();
    checkLayerCall(layerEndFrame(session, &endInfo), "xrEndFrame");
    state.mGameEndInfo = nullptr;
    Check(
      state.mFrames.size() == framesBefore + 1,
      "the layer did not call the runtime's xrEndFrame exactly once");
    state.mFrames.back().mActiveInGameViewID =
      SHM::ActiveConsumers::Get().mActiveViewID;
  }
  state.mStep = nullptr;

  checkLayerCall(layerDestroySession(session), "xrDestroySession");
  checkLayerCall(layerDestroyInstance(instance), "xrDestroyInstance");
  return std::move(state.mFrames);
}

}// namespace OpenKneeboard::Tests
