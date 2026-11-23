//! The D3D11 boundary: SHM texture mapping, device-context state, the sprite
//! batch and the renderer (C++ `SHM::D3D11`, `D3D11::*`).

mod renderer;
mod shm_client;
mod sprite_batch;
mod state;
#[cfg(test)]
mod tests;

pub use renderer::{Renderer, SwapchainImage};
pub use shm_client::{MappedFrame, ShmClient};
pub use state::DeviceContextState;
