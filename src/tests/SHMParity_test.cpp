// OpenKneeboard
//
// Copyright (c) 2026 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.
#include <OpenKneeboard/SHM.hpp>
#include <OpenKneeboard/SHM/ActiveConsumers.hpp>
#include <OpenKneeboard/SHM/Wire.hpp>

#include <OpenKneeboard/config.hpp>
#include <OpenKneeboard/version.hpp>

#include <shims/winrt/base.h>

#include <Windows.h>

#include <gtest/gtest.h>

#include <algorithm>
#include <array>
#include <bit>
#include <cstddef>
#include <cstring>
#include <format>
#include <memory>
#include <span>
#include <string>
#include <utility>
#include <variant>
#include <vector>

#include <okb_ffi.h>

using namespace OpenKneeboard;

static_assert(sizeof(OkbConfig) == sizeof(SHM::Config));
static_assert(alignof(OkbConfig) == alignof(SHM::Config));
static_assert(sizeof(OkbLayerConfig) == sizeof(SHM::LayerConfig));
static_assert(alignof(OkbLayerConfig) == alignof(SHM::LayerConfig));

namespace {

struct NamesDeleter {
  void operator()(OkbShmNames* p) const { okb_shm_names_free(p); }
};
struct WriterDeleter {
  void operator()(OkbShmWriter* p) const { okb_shm_writer_free(p); }
};
struct ReaderDeleter {
  void operator()(OkbShmReader* p) const { okb_shm_reader_free(p); }
};

// Test-only reverse domain, so this never touches a running OpenKneeboard.
std::unique_ptr<OkbShmNames, NamesDeleter> TestNames() {
  const auto domain = std::format("okb-test.ffi.{}", GetCurrentProcessId());
  return std::unique_ptr<OkbShmNames, NamesDeleter> {
    okb_shm_names_new(domain.c_str(), 1, 0, 0, 0)};
}

constexpr uint64_t Gpu = 1;
constexpr uint32_t ViewerKind = 0xFFFFFFFF;// SHM::ConsumerKind::Viewer

}// namespace

TEST(SHMFfi, WriterReaderRoundTrip) {
  const auto names = TestNames();
  ASSERT_NE(names, nullptr);

  std::unique_ptr<OkbShmWriter, WriterDeleter> writer {
    okb_shm_writer_new(names.get(), Gpu)};
  ASSERT_NE(writer, nullptr);

  int sessionChanges = 0;
  std::unique_ptr<OkbShmReader, ReaderDeleter> reader {okb_shm_reader_new(
    names.get(),
    ViewerKind,
    Gpu,
    [](void* ctx) { ++*static_cast<int*>(ctx); },
    &sessionChanges)};
  ASSERT_NE(reader, nullptr);
  EXPECT_FALSE(okb_shm_reader_has_feeder(reader.get()));

  // Zero-layer frame: sets the feeder-attached flag, but has no layers.
  const SHM::Config config {};
  okb_shm_writer_lock(writer.get());
  const auto info = okb_shm_writer_begin_frame(writer.get());
  okb_shm_writer_submit_frame(
    writer.get(),
    info,
    reinterpret_cast<const OkbConfig*>(&config),
    nullptr,
    0,
    0,
    0);
  EXPECT_EQ(okb_shm_writer_frame_count(writer.get()), 1);
  okb_shm_writer_unlock(writer.get());
  EXPECT_EQ(okb_shm_writer_frame_count(writer.get()), 1);// unlocked, as C++

  EXPECT_TRUE(okb_shm_reader_has_feeder(reader.get()));
  OkbFrame frame {};
  EXPECT_EQ(okb_shm_reader_maybe_get(reader.get(), &frame), 3);// NoLayers
  EXPECT_EQ(sessionChanges, 1);

  EXPECT_TRUE(okb_shm_writer_try_lock(writer.get()));
  okb_shm_writer_detach(writer.get());
  okb_shm_writer_unlock(writer.get());
  EXPECT_FALSE(okb_shm_reader_has_feeder(reader.get()));
  EXPECT_EQ(okb_shm_reader_maybe_get(reader.get(), &frame), 2);// NoFeeder
}

// Differential parity harness: each script runs with every C++/Rust
// writer x reader pairing, and every trace must equal the C++/C++ one.
// See docs/internals/rust-port-parity-contract.md.
namespace {

namespace Wire = SHM::Wire;

constexpr uint64_t WriterGpu = 0x1234;
constexpr uint64_t OtherGpu = 0x5678;

enum class Impl { Cpp, Rust };

const char* Name(Impl impl) { return impl == Impl::Cpp ? "C++" : "Rust"; }

struct Lock {};
struct Unlock {};
struct SubmitEmptyFrame {};
struct SubmitFrame {
  SHM::Config mConfig {};
  std::vector<SHM::LayerConfig> mLayers;
  bool mUseHandles {false};
};
struct Detach {};
struct Read {
  std::size_t mReaderIndex {};
};
using Step =
  std::variant<Lock, Unlock, SubmitEmptyFrame, SubmitFrame, Detach, Read>;

struct ReaderSpec {
  SHM::ConsumerKind mKind {SHM::ConsumerKind::Viewer};
  uint64_t mGpu {WriterGpu};
};
constexpr ReaderSpec DefaultReaders[] {{}};

// One MaybeGet(), normalized across implementations.
struct ReadResult {
  std::size_t mStep {};// for messages; not compared
  bool mHasFeeder {};// operator bool, before the read
  int mCode {};// 0, 1 + SHM::Frame::Error, or -(Win32 error)
  SHM::Config mConfig {};
  std::vector<SHM::LayerConfig> mLayers;
  int64_t mFenceIn {};
  uint8_t mIndex {};
  // Index into Events() of the object the handle refers to; -1 if unusable.
  int mTexture {-1};
  int mFence {-1};
};

using Snapshot = std::array<std::byte, Wire::SHM_SIZE>;

struct Trace {
  std::vector<ReadResult> reads;
  std::vector<Snapshot> snapshots;// after every step
  std::vector<int> sessionCallbacks;// reader's total, after every read
};

// Shared by all pairings so the handle values written to the SHM match.
// SubmitFrame number N (0-based) with handles uses events 2N and 2N + 1.
constexpr std::size_t EventCount = 16;
std::span<const winrt::handle> Events() {
  static const auto sEvents = [] {
    std::array<winrt::handle, EventCount> ret;
    for (auto& event: ret) {
      event.attach(CreateEventW(nullptr, TRUE, FALSE, nullptr));
    }
    return ret;
  }();
  return sEvents;
}

int EventIndex(HANDLE handle) {
  if (!handle) {
    return -1;
  }
  const auto events = Events();
  for (std::size_t i = 0; i < events.size(); ++i) {
    if (CompareObjectHandles(handle, events[i].get())) {
      return static_cast<int>(i);
    }
  }
  return -1;
}

const OkbShmNames* AppNames() {
  static const std::unique_ptr<OkbShmNames, NamesDeleter> sNames {
    okb_shm_names_new(
      ProjectReverseDomainA,
      Version::Major,
      Version::Minor,
      Version::Patch,
      Version::Build)};
  return sNames.get();
}

// As SHMPath() in src/lib/SHM.cpp
std::wstring AppSHMPath() {
  return std::format(
    L"{}/{}.{}.{}.{}-s{:x}",
    ProjectReverseDomainW,
    Version::Major,
    Version::Minor,
    Version::Patch,
    Version::Build,
    Wire::SHM_SIZE);
}

class ScriptWriter {
 public:
  virtual ~ScriptWriter() = default;
  virtual bool valid() const = 0;
  virtual void lock() = 0;
  virtual void unlock() = 0;
  virtual void submitEmpty() = 0;
  virtual void submit(
    const SHM::Config&,
    const std::vector<SHM::LayerConfig>&,
    HANDLE texture,
    HANDLE fence) = 0;
  virtual void detach() = 0;
};

class CppWriter final : public ScriptWriter {
 public:
  bool valid() const override { return static_cast<bool>(mWriter); }
  void lock() override { mWriter.lock(); }
  void unlock() override { mWriter.unlock(); }
  void submitEmpty() override { mWriter.SubmitEmptyFrame(); }
  void submit(
    const SHM::Config& config,
    const std::vector<SHM::LayerConfig>& layers,
    HANDLE texture,
    HANDLE fence) override {
    const auto info = mWriter.BeginFrame();
    mWriter.SubmitFrame(info, config, layers, texture, fence);
  }
  void detach() override { mWriter.Detach(); }

 private:
  SHM::Writer mWriter {WriterGpu};
};

class RustWriter final : public ScriptWriter {
 public:
  bool valid() const override { return static_cast<bool>(mWriter); }
  void lock() override { okb_shm_writer_lock(mWriter.get()); }
  void unlock() override { okb_shm_writer_unlock(mWriter.get()); }
  void submitEmpty() override {
    okb_shm_writer_submit_empty_frame(mWriter.get());
  }
  void submit(
    const SHM::Config& config,
    const std::vector<SHM::LayerConfig>& layers,
    HANDLE texture,
    HANDLE fence) override {
    const auto info = okb_shm_writer_begin_frame(mWriter.get());
    okb_shm_writer_submit_frame(
      mWriter.get(),
      info,
      reinterpret_cast<const OkbConfig*>(&config),
      reinterpret_cast<const OkbLayerConfig*>(layers.data()),
      layers.size(),
      reinterpret_cast<uintptr_t>(texture),
      reinterpret_cast<uintptr_t>(fence));
  }
  void detach() override { okb_shm_writer_detach(mWriter.get()); }

 private:
  std::unique_ptr<OkbShmWriter, WriterDeleter> mWriter {
    okb_shm_writer_new(AppNames(), WriterGpu)};
};

class ScriptReader {
 public:
  virtual ~ScriptReader() = default;
  virtual ReadResult read() = 0;
  virtual int sessionChanges() const = 0;
};

class CountingReader final : public SHM::Reader {
 public:
  using SHM::Reader::Reader;
  int mSessionChanges {0};

 protected:
  void OnSessionChanged() override { ++mSessionChanges; }
};

class CppReader final : public ScriptReader {
 public:
  explicit CppReader(const ReaderSpec& spec) : mReader(spec.mKind, spec.mGpu) {}

  ReadResult read() override {
    ReadResult ret {.mHasFeeder = static_cast<bool>(mReader)};
    try {
      auto frame = mReader.MaybeGet();
      if (!frame) {
        ret.mCode = 1 + static_cast<int>(frame.error());
        return ret;
      }
      ret.mConfig = frame->mConfig;
      ret.mLayers.assign(frame->mLayers.begin(), frame->mLayers.end());
      ret.mFenceIn = frame->mFenceIn;
      ret.mIndex = frame->mIndex;
      ret.mTexture = EventIndex(frame->mTexture);
      ret.mFence = EventIndex(frame->mFence);
    } catch (const winrt::hresult_error& e) {
      // DuplicateHandle failure: winrt::check_bool(), HRESULT_FROM_WIN32
      ret.mCode = -static_cast<int>(HRESULT_CODE(e.code()));
    }
    return ret;
  }

  int sessionChanges() const override { return mReader.mSessionChanges; }

 private:
  CountingReader mReader;
};

class RustReader final : public ScriptReader {
 public:
  explicit RustReader(const ReaderSpec& spec)
    : mReader(okb_shm_reader_new(
        AppNames(),
        static_cast<uint32_t>(spec.mKind),
        spec.mGpu,
        [](void* ctx) { ++*static_cast<int*>(ctx); },
        &mSessionChanges)) {}

  ReadResult read() override {
    ReadResult ret {.mHasFeeder = okb_shm_reader_has_feeder(mReader.get())};
    OkbFrame frame {};
    ret.mCode = okb_shm_reader_maybe_get(mReader.get(), &frame);
    if (ret.mCode != 0) {
      return ret;
    }
    ret.mConfig = std::bit_cast<SHM::Config>(frame.config);
    const auto layers = reinterpret_cast<const SHM::LayerConfig*>(frame.layers);
    ret.mLayers.assign(layers, layers + frame.layer_count);
    ret.mFenceIn = frame.fence_in;
    ret.mIndex = frame.index;
    ret.mTexture = EventIndex(reinterpret_cast<HANDLE>(frame.texture));
    ret.mFence = EventIndex(reinterpret_cast<HANDLE>(frame.fence));
    return ret;
  }

  int sessionChanges() const override { return mSessionChanges; }

 private:
  int mSessionChanges {0};// declared first: mReader's callback points here
  std::unique_ptr<OkbShmReader, ReaderDeleter> mReader;
};

// Runs one writer/reader pairing under the real app names; the writer is
// destroyed (and detaches) before this returns.
Trace RunScript(
  Impl writerImpl,
  Impl readerImpl,
  std::span<const Step> steps,
  std::span<const ReaderSpec> readerSpecs = DefaultReaders) {
  std::unique_ptr<ScriptWriter> writer;
  if (writerImpl == Impl::Cpp) {
    writer = std::make_unique<CppWriter>();
  } else {
    writer = std::make_unique<RustWriter>();
  }
  if (!writer->valid()) {
    ADD_FAILURE() << Name(writerImpl) << " writer unavailable";
    return {};
  }
  std::vector<std::unique_ptr<ScriptReader>> readers;
  for (const auto& spec: readerSpecs) {
    if (readerImpl == Impl::Cpp) {
      readers.push_back(std::make_unique<CppReader>(spec));
    } else {
      readers.push_back(std::make_unique<RustReader>(spec));
    }
  }

  const winrt::handle mapping {
    OpenFileMappingW(FILE_MAP_READ, FALSE, AppSHMPath().c_str())};
  if (!mapping) {
    ADD_FAILURE() << "OpenFileMappingW failed: " << GetLastError();
    return {};
  }
  const std::unique_ptr<void, decltype(&UnmapViewOfFile)> view {
    MapViewOfFile(mapping.get(), FILE_MAP_READ, 0, 0, Wire::SHM_SIZE),
    &UnmapViewOfFile};
  if (!view) {
    ADD_FAILURE() << "MapViewOfFile failed: " << GetLastError();
    return {};
  }

  Trace trace;
  std::size_t submitCount = 0;
  for (std::size_t i = 0; i < steps.size(); ++i) {
    if (const auto read = std::get_if<Read>(&steps[i])) {
      auto& reader = *readers.at(read->mReaderIndex);
      auto result = reader.read();
      result.mStep = i;
      trace.reads.push_back(std::move(result));
      trace.sessionCallbacks.push_back(reader.sessionChanges());
    } else if (std::holds_alternative<Lock>(steps[i])) {
      writer->lock();
    } else if (std::holds_alternative<Unlock>(steps[i])) {
      writer->unlock();
    } else if (std::holds_alternative<SubmitEmptyFrame>(steps[i])) {
      writer->submitEmpty();
    } else if (std::holds_alternative<Detach>(steps[i])) {
      writer->detach();
    } else {
      const auto& submit = std::get<SubmitFrame>(steps[i]);
      const auto n = submitCount++;
      HANDLE texture {};
      HANDLE fence {};
      if (submit.mUseHandles) {
        texture = Events()[2 * n].get();
        fence = Events()[(2 * n) + 1].get();
      }
      writer->submit(submit.mConfig, submit.mLayers, texture, fence);
    }
    auto& snapshot = trace.snapshots.emplace_back();
    std::memcpy(snapshot.data(), view.get(), snapshot.size());
  }
  return trace;
}

// Names the SharedData field holding byte `offset`, e.g.
// "mFrames[1].mLayers[2]+12".
std::string DescribeOffset(std::size_t offset) {
  using SD = Wire::SharedData;
  using F = SD::Frame;
  using Field = std::pair<std::size_t, const char*>;
  const auto find = [](std::span<const Field> fields, std::size_t at) {
    return *std::ranges::find_if(
      fields.rbegin(), fields.rend(), [=](const auto& f) {
        return f.first <= at;
      });
  };
  constexpr Field header[] {
    {offsetof(SD, mMagic), "mMagic"},
    {offsetof(SD, mGPULUID), "mGPULUID"},
    {offsetof(SD, mFrameNumber), "mFrameNumber"},
    {offsetof(SD, mSessionID), "mSessionID"},
    {offsetof(SD, mFlags), "mFlags"},
    {offsetof(SD, mFeederProcessID), "mFeederProcessID"},
    {offsetof(SD, mPadding), "mPadding"},
  };
  if (offset < offsetof(SD, mFrames)) {
    const auto [start, name] = find(header, offset);
    return std::format("{}+{}", name, offset - start);
  }
  constexpr Field frameFields[] {
    {offsetof(F, mTexture), "mTexture"},
    {offsetof(F, mFence), "mFence"},
    {offsetof(F, mReadyReadFenceValue), "mReadyReadFenceValue"},
    {offsetof(F, mConfig), "mConfig"},
    {offsetof(F, mLayers), "mLayers"},
    {offsetof(F, mLayerCount), "mLayerCount"},
  };
  const auto inFrames = offset - offsetof(SD, mFrames);
  const auto frame = inFrames / sizeof(F);
  auto inFrame = inFrames % sizeof(F);
  const auto [start, name] = find(frameFields, inFrame);
  inFrame -= start;
  if (start == offsetof(F, mLayers) && inFrame < sizeof(F::mLayers)) {
    return std::format(
      "mFrames[{}].mLayers[{}]+{}",
      frame,
      inFrame / sizeof(SHM::LayerConfig),
      inFrame % sizeof(SHM::LayerConfig));
  }
  return std::format("mFrames[{}].{}+{}", frame, name, inFrame);
}

void ExpectSameRead(
  const ReadResult& e,
  const ReadResult& a,
  const std::string& where) {
  EXPECT_EQ(e.mHasFeeder, a.mHasFeeder) << where << ": operator bool";
  EXPECT_EQ(e.mCode, a.mCode) << where << ": result code";
  EXPECT_EQ(e.mConfig.mGlobalInputLayerID, a.mConfig.mGlobalInputLayerID)
    << where << ": mConfig.mGlobalInputLayerID";
  EXPECT_EQ(e.mConfig.mVR, a.mConfig.mVR) << where << ": mConfig.mVR";
  EXPECT_EQ(e.mConfig.mTextureSize, a.mConfig.mTextureSize)
    << where << ": mConfig.mTextureSize";
  EXPECT_EQ(e.mConfig.mTint, a.mConfig.mTint) << where << ": mConfig.mTint";
  EXPECT_EQ(e.mLayers.size(), a.mLayers.size()) << where << ": layer count";
  for (std::size_t i = 0; i < std::min(e.mLayers.size(), a.mLayers.size());
       ++i) {
    const auto& el = e.mLayers[i];
    const auto& al = a.mLayers[i];
    const auto layer = std::format("{}: mLayers[{}]", where, i);
    EXPECT_EQ(el.mLayerID, al.mLayerID) << layer << ".mLayerID";
    EXPECT_EQ(el.mVREnabled, al.mVREnabled) << layer << ".mVREnabled";
    EXPECT_EQ(el.mVR.mPose, al.mVR.mPose) << layer << ".mVR.mPose";
    EXPECT_EQ(el.mVR.mPhysicalSize, al.mVR.mPhysicalSize)
      << layer << ".mVR.mPhysicalSize";
    EXPECT_EQ(el.mVR.mEnableGazeZoom, al.mVR.mEnableGazeZoom)
      << layer << ".mVR.mEnableGazeZoom";
    EXPECT_EQ(el.mVR.mZoomScale, al.mVR.mZoomScale)
      << layer << ".mVR.mZoomScale";
    EXPECT_EQ(el.mVR.mGazeTargetScale, al.mVR.mGazeTargetScale)
      << layer << ".mVR.mGazeTargetScale";
    EXPECT_EQ(el.mVR.mOpacity, al.mVR.mOpacity) << layer << ".mVR.mOpacity";
    EXPECT_EQ(el.mVR.mLocationOnTexture, al.mVR.mLocationOnTexture)
      << layer << ".mVR.mLocationOnTexture";
  }
  EXPECT_EQ(e.mFenceIn, a.mFenceIn) << where << ": mFenceIn";
  EXPECT_EQ(e.mIndex, a.mIndex) << where << ": mIndex";
  EXPECT_EQ(e.mTexture, a.mTexture) << where << ": mTexture (event index)";
  EXPECT_EQ(e.mFence, a.mFence) << where << ": mFence (event index)";
}

// Snapshot bytes excluded from comparison; nothing else is masked.
// - mSessionID: random per writer and per Detach().
// - The trailing padding of each SharedData::Frame slot: C++
//   Writer::SubmitFrame copies a temporary whose padding is indeterminate
//   (0xCC in Debug builds, stack data in Release); Rust writes zeros.
constexpr auto SessionBegin = offsetof(Wire::SharedData, mSessionID);
constexpr auto SessionEnd = SessionBegin + sizeof(Wire::SharedData::mSessionID);
using WireFrame = Wire::SharedData::Frame;
constexpr auto FrameTailBegin =
  offsetof(WireFrame, mLayerCount) + sizeof(WireFrame::mLayerCount);
static_assert(
  sizeof(WireFrame) - FrameTailBegin == 4,
  "Frame layout changed: revisit the trailing-padding mask");

bool IsMasked(std::size_t offset) {
  if (offset >= SessionBegin && offset < SessionEnd) {
    return true;
  }
  constexpr auto framesBegin = offsetof(Wire::SharedData, mFrames);
  for (std::size_t slot = 0; slot < SHM::SwapChainLength; ++slot) {
    const auto begin = framesBegin + (slot * sizeof(WireFrame)) + FrameTailBegin;
    const auto end = framesBegin + ((slot + 1) * sizeof(WireFrame));
    if (offset >= begin && offset < end) {
      return true;
    }
  }
  return false;
}

void ExpectSameTrace(const Trace& expected, const Trace& actual) {
  ASSERT_EQ(expected.reads.size(), actual.reads.size());
  ASSERT_EQ(expected.snapshots.size(), actual.snapshots.size());
  ASSERT_EQ(expected.sessionCallbacks.size(), actual.sessionCallbacks.size());

  for (std::size_t i = 0; i < expected.reads.size(); ++i) {
    const auto where =
      std::format("read {} (step {})", i, expected.reads[i].mStep);
    ExpectSameRead(expected.reads[i], actual.reads[i], where);
    EXPECT_EQ(expected.sessionCallbacks[i], actual.sessionCallbacks[i])
      << where << ": session callbacks so far";
  }

  for (std::size_t step = 0; step < expected.snapshots.size(); ++step) {
    const auto& e = expected.snapshots[step];
    const auto& a = actual.snapshots[step];
    for (std::size_t i = 0; i < e.size(); ++i) {
      if (IsMasked(i)) {
        continue;
      }
      if (e[i] != a[i]) {
        ADD_FAILURE() << std::format(
          "SHM bytes after step {} first differ at offset {} ({}): expected "
          "{:#04x}, got {:#04x}",
          step,
          i,
          DescribeOffset(i),
          static_cast<int>(e[i]),
          static_cast<int>(a[i]));
        break;
      }
    }
  }
}

// Runs `steps` in all four pairings; returns the C++/C++ trace so tests can
// also pin the reference behavior.
Trace ExpectParity(
  std::span<const Step> steps,
  std::span<const ReaderSpec> readers = DefaultReaders) {
  auto reference = RunScript(Impl::Cpp, Impl::Cpp, steps, readers);
  constexpr std::pair<Impl, Impl> pairings[] {
    {Impl::Cpp, Impl::Rust},
    {Impl::Rust, Impl::Cpp},
    {Impl::Rust, Impl::Rust},
  };
  for (const auto [w, r]: pairings) {
    SCOPED_TRACE(
      std::format("{} writer, {} reader vs C++/C++", Name(w), Name(r)));
    ExpectSameTrace(reference, RunScript(w, r, steps, readers));
  }
  return reference;
}

SHM::LayerConfig MakeLayer(uint64_t id, float offset) {
  SHM::LayerConfig layer {.mLayerID = id, .mVREnabled = true};
  layer.mVR.mPose = {
    .mX = offset,
    .mEyeY = -offset,
    .mZ = 2 * offset,
    .mRX = offset / 3,
    .mRY = -offset / 5,
    .mRZ = offset / 7,
  };
  layer.mVR.mPhysicalSize = {0.25f + offset, 0.5f + offset};
  layer.mVR.mEnableGazeZoom = (id % 2) == 0;
  layer.mVR.mZoomScale = 1.5f + offset;
  layer.mVR.mGazeTargetScale = {1.0f + offset, 2.0f - offset};
  layer.mVR.mOpacity = {0.75f - offset, 0.5f + offset};
  const auto px = static_cast<uint32_t>(id);
  layer.mVR.mLocationOnTexture = {
    {px * 100, px * 10},
    {320 + px, 240 + px},
    (id % 2) ? PixelRect::Origin::BottomLeft : PixelRect::Origin::TopLeft,
  };
  return layer;
}

SHM::Config MakeConfig(uint64_t globalInputLayerID) {
  SHM::Config config {
    .mGlobalInputLayerID = globalInputLayerID,
    .mTextureSize = {2048, 1024},
    .mTint = {0.9f, 0.8f, 0.7f, 0.6f},
  };
  config.mVR.mQuirks.mOpenXR_Upscaling =
    VRRenderSettings::Quirks::Upscaling::AlwaysOn;
  config.mVR.mEnableGazeInputFocus = false;
  config.mVR.mForceZoom = true;
  config.mVR.mRecenterCount = 3;
  return config;
}

SubmitFrame OneLayerFrame() {
  return {MakeConfig(1), {MakeLayer(1, 0.1f)}, true};
}

class SHMParity : public ::testing::Test {
 protected:
  void SetUp() override {
    CountingReader probe(SHM::ConsumerKind::Viewer, WriterGpu);
    if (static_cast<bool>(probe)) {
      GTEST_SKIP()
        << "OpenKneeboard appears to be running; close it to run SHM tests";
    }
  }
};

}// namespace

TEST_F(SHMParity, EmptyAndZeroLayerFrames) {
  const std::vector<Step> steps {
    Read {},// writer constructed, nothing submitted: NoFeeder
    Lock {},
    SubmitEmptyFrame {},
    Unlock {},
    Read {},// empty frame never attaches the feeder: NoFeeder
    Lock {},
    SubmitFrame {},
    Unlock {},
    Read {},// NoLayers
    Read {},
    Lock {},
    SubmitEmptyFrame {},
    Unlock {},
    Read {},// attached, empty slot: NoLayers
  };
  const auto ref = ExpectParity(steps);
  ASSERT_EQ(ref.reads.size(), 5);
  EXPECT_EQ(ref.reads[0].mCode, 2);
  EXPECT_EQ(ref.reads[1].mCode, 2);
  EXPECT_EQ(ref.reads[2].mCode, 3);
  EXPECT_EQ(ref.reads[4].mCode, 3);
}

TEST_F(SHMParity, MultiLayerFrame) {
  const std::vector<Step> steps {
    Lock {},
    SubmitFrame {
      MakeConfig(2),
      {MakeLayer(1, 0.1f), MakeLayer(2, 0.2f), MakeLayer(3, 0.3f)},
      true,
    },
    Unlock {},
    Read {},
    Read {},// same foreign handles: no re-duplication
    Lock {},
    // Layers but null handles: DuplicateHandle fails (C++ throws)
    SubmitFrame {MakeConfig(2), {MakeLayer(4, 0.4f)}, false},
    Unlock {},
    Read {},
  };
  const auto ref = ExpectParity(steps);
  ASSERT_EQ(ref.reads.size(), 3);
  EXPECT_EQ(ref.reads[0].mCode, 0);
  EXPECT_EQ(ref.reads[0].mLayers.size(), 3);
  EXPECT_EQ(ref.reads[0].mTexture, 0);
  EXPECT_EQ(ref.reads[0].mFence, 1);
  EXPECT_EQ(ref.reads[2].mCode, -ERROR_INVALID_HANDLE);
}

TEST_F(SHMParity, SwapchainWrapAround) {
  std::vector<Step> steps;
  for (uint64_t i = 0; i < 5; ++i) {
    std::vector<SHM::LayerConfig> layers;
    for (uint64_t j = 0; j <= i % 3; ++j) {
      layers.push_back(MakeLayer(j + 1, 0.1f * static_cast<float>(i + j)));
    }
    steps.insert(
      steps.end(),
      {Lock {}, SubmitFrame {MakeConfig(i), layers, true}, Unlock {}, Read {}});
  }
  const auto ref = ExpectParity(steps);
  ASSERT_EQ(ref.reads.size(), 5);
  for (std::size_t i = 0; i < 5; ++i) {
    EXPECT_EQ(ref.reads[i].mCode, 0) << i;
    EXPECT_EQ(ref.reads[i].mIndex, (i + 1) % 2) << i;
    EXPECT_EQ(ref.reads[i].mTexture, 2 * i) << i;
  }
}

TEST_F(SHMParity, WrongGpu) {
  constexpr ReaderSpec readers[] {
    {SHM::ConsumerKind::Viewer, WriterGpu},
    {SHM::ConsumerKind::Viewer, OtherGpu},
  };
  const std::vector<Step> steps {
    Lock {},
    SubmitFrame {},
    Unlock {},
    Read {1},// IncorrectGPU, even with no layers
    Read {0},// NoLayers
    Lock {},
    OneLayerFrame(),
    Unlock {},
    Read {1},
    Read {0},
  };
  const auto ref = ExpectParity(steps, readers);
  ASSERT_EQ(ref.reads.size(), 4);
  EXPECT_EQ(ref.reads[0].mCode, 1);
  EXPECT_EQ(ref.reads[1].mCode, 3);
  EXPECT_EQ(ref.reads[2].mCode, 1);
  EXPECT_EQ(ref.reads[3].mCode, 0);
}

TEST_F(SHMParity, DetachAndReattach) {
  const std::vector<Step> steps {
    Lock {},
    OneLayerFrame(),
    Unlock {},
    Read {},
    Lock {},
    Detach {},
    Unlock {},
    Read {},// NoFeeder; no session callback
    Lock {},
    SubmitEmptyFrame {},
    Unlock {},
    Read {},// still NoFeeder
    Lock {},
    OneLayerFrame(),// new session, new handles
    Unlock {},
    Read {},
  };
  const auto ref = ExpectParity(steps);
  ASSERT_EQ(ref.reads.size(), 4);
  EXPECT_EQ(ref.reads[0].mCode, 0);
  EXPECT_EQ(ref.reads[1].mCode, 2);
  EXPECT_EQ(ref.reads[2].mCode, 2);
  EXPECT_EQ(ref.reads[3].mCode, 0);
  EXPECT_EQ(ref.reads[3].mTexture, 2);
  EXPECT_EQ(ref.sessionCallbacks, (std::vector {1, 1, 1, 2}));
}

TEST_F(SHMParity, ActiveConsumers) {
  const std::vector<Step> steps {Lock {}, OneLayerFrame(), Unlock {}, Read {}};
  using AC = SHM::ActiveConsumers;
  const auto fieldsSet = [&](Impl w, Impl r, SHM::ConsumerKind kind) {
    AC::Clear();
    const ReaderSpec readers[] {{kind, WriterGpu}};
    const auto trace = RunScript(w, r, steps, readers);
    EXPECT_EQ(trace.reads.size(), 1);
    if (!trace.reads.empty()) {
      EXPECT_EQ(trace.reads[0].mCode, 0);
    }
    const auto c = AC::Get();
    std::vector<std::string> ret;
    for (const auto& [value, name]: {
           std::pair {c.mOpenVR, "mOpenVR"},
           std::pair {c.mOpenXR_D3D11, "mOpenXR_D3D11"},
           std::pair {c.mOpenXR_D3D12, "mOpenXR_D3D12"},
           std::pair {c.mOpenXR_Vulkan2, "mOpenXR_Vulkan2"},
           std::pair {c.mViewer, "mViewer"},
         }) {
      if (value != AC::T {}) {
        ret.push_back(name);
      }
    }
    return ret;
  };

  constexpr std::pair<SHM::ConsumerKind, const char*> kinds[] {
    {SHM::ConsumerKind::OpenVR, "OpenVR"},
    {SHM::ConsumerKind::OpenXR_D3D11, "OpenXR_D3D11"},
    {SHM::ConsumerKind::OpenXR_D3D12, "OpenXR_D3D12"},
    {SHM::ConsumerKind::OpenXR_Vulkan2, "OpenXR_Vulkan2"},
    {SHM::ConsumerKind::Viewer, "Viewer"},
  };
  constexpr std::pair<Impl, Impl> pairings[] {
    {Impl::Cpp, Impl::Rust},
    {Impl::Rust, Impl::Cpp},
    {Impl::Rust, Impl::Rust},
  };
  for (const auto [kind, kindName]: kinds) {
    const auto expected = fieldsSet(Impl::Cpp, Impl::Cpp, kind);
    EXPECT_EQ(expected.size(), 1) << kindName;
    for (const auto [w, r]: pairings) {
      EXPECT_EQ(fieldsSet(w, r, kind), expected) << std::format(
        "{}: {} writer, {} reader vs C++/C++", kindName, Name(w), Name(r));
    }
  }
  // Known quirk: D3D12 consumers stamp the D3D11 field.
  EXPECT_EQ(
    fieldsSet(Impl::Cpp, Impl::Cpp, SHM::ConsumerKind::OpenXR_D3D12),
    std::vector<std::string> {"mOpenXR_D3D11"});
}
