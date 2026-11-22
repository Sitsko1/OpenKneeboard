//! OpenKneeboard shared-memory (SHM) wire protocol.

/// Number of frame slots in the SHM swapchain.
///
/// Mirrors `Config::SHMSwapchainLength` in
/// `src/lib/include/OpenKneeboard/config.in.hpp`.
pub const SWAPCHAIN_LENGTH: usize = 2;

/// Maximum number of views (kneeboard layers) in one SHM frame.
///
/// Mirrors `MaxViewCount` in `src/lib/include/OpenKneeboard/config.in.hpp`.
pub const MAX_VIEW_COUNT: usize = 16;

pub mod active_consumers;
mod names;
pub mod wire;
pub use active_consumers::ConsumerKind;
pub use names::{Names, Version};
mod reader;
pub use reader::{Frame, FrameError, ReadError, Reader};
mod win32;
mod writer;
pub use writer::{NextFrameInfo, Writer, WriterGuard};
