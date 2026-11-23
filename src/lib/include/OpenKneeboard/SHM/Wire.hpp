// OpenKneeboard
//
// Copyright (c) 2025 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.
#pragma once

#include <OpenKneeboard/SHM.hpp>

#include <OpenKneeboard/bitflags.hpp>
#include <OpenKneeboard/config.hpp>

#include <Windows.h>

#include <bit>
#include <cstdint>
#include <string_view>
#include <type_traits>

namespace OpenKneeboard::SHM::Wire {

enum class HeaderFlags : ULONG {
  FEEDER_ATTACHED = 1 << 0,
};

}// namespace OpenKneeboard::SHM::Wire

namespace OpenKneeboard {
template <>
constexpr bool is_bitflags_v<SHM::Wire::HeaderFlags> = true;
}// namespace OpenKneeboard

namespace OpenKneeboard::SHM::Wire {

// Defined in SHM.cpp
uint64_t CreateSessionID();

/* Wrapper for a HANDLE that is always 64-bits large
 *
 * From
 * https://learn.microsoft.com/en-us/windows/win32/winprog64/interprocess-communication:
 *
 * > 64-bit versions of Windows use 32-bit handles for interoperability. When
 * > sharing a handle between 32-bit and 64-bit applications, only the lower 32
 * > bits are significant, so it is safe to truncate the handle (when passing it
 * > from 64-bit to 32-bit) or sign-extend the handle (when passing it from
 * > 32-bit to 64-bit).
 */
class FixedSizeHandle final {
 public:
  FixedSizeHandle() noexcept = default;

  FixedSizeHandle(HANDLE value) noexcept
    : mValue(std::bit_cast<uintptr_t>(value)) {}

  operator HANDLE() const noexcept {
    return std::bit_cast<HANDLE>(static_cast<uintptr_t>(mValue));
  }

 private:
  uint64_t mValue {};
};
static_assert(sizeof(FixedSizeHandle) == sizeof(uint64_t));

struct SharedData final {
  struct Frame {
    FixedSizeHandle mTexture {nullptr};
    FixedSizeHandle mFence {nullptr};
    int64_t mReadyReadFenceValue {};

    Config mConfig {};
    LayerConfig mLayers[MaxViewCount];

    uint32_t mLayerCount {};
  };

  // Use the magic string to make sure we don't have
  // uninitialized memory that happens to have the
  // feeder-attached bit set
  static constexpr std::string_view Magic {"OKBMagic"};
  static_assert(Magic.size() == sizeof(uint64_t));
  uint64_t mMagic = *reinterpret_cast<const uint64_t*>(Magic.data());

  uint64_t mGPULUID {};

  uint64_t mFrameNumber = 0;
  uint64_t mSessionID = CreateSessionID();
  HeaderFlags mFlags;

  DWORD mFeederProcessID {};

  uint8_t mPadding[8];
  Frame mFrames[SwapChainLength];

  [[nodiscard]]
  bool HaveFeeder() const;
};

static_assert(std::is_standard_layout_v<SharedData>);
constexpr DWORD SHM_SIZE = sizeof(SharedData);
static_assert(
  SHM_SIZE == 3024,
  "Potential mismatch between 32-bit and 64-bit SHM layout");

}// namespace OpenKneeboard::SHM::Wire
