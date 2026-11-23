//! C++ `D3D11::Renderer` (`src/lib/D3D11/Renderer.cpp`).
#![allow(unsafe_code)]

use okb_shm::wire::PixelSize;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext4, ID3D11RenderTargetView, ID3D11Texture2D,
};
use windows::core::Interface;

use super::MappedFrame;
use super::sprite_batch::SpriteBatch;
use crate::compose::LayerSprite;

/// C++ `SwapchainBufferResources`.
pub struct SwapchainImage {
    pub texture: ID3D11Texture2D,
    pub rtv: ID3D11RenderTargetView,
}

pub struct Renderer {
    sprite_batch: SpriteBatch,
    /// `ID3D11DeviceContext4` for `ID3D11Fence` support.
    ctx: ID3D11DeviceContext4,
}

impl Renderer {
    /// # Panics
    /// On HRESULT failure, or if the context lacks `ID3D11DeviceContext4`.
    pub fn new(device: &ID3D11Device) -> Self {
        // SAFETY: plain COM call on a live device.
        let ctx = unsafe { device.GetImmediateContext() }.expect("GetImmediateContext");
        Self {
            ctx: ctx.cast().expect("ID3D11DeviceContext4"),
            sprite_batch: SpriteBatch::new(device),
        }
    }

    /// C++ `RenderLayers` with `RenderMode::ClearAndRender`.
    ///
    /// # Panics
    /// On HRESULT failure.
    pub fn render_layers(
        &mut self,
        dest: &SwapchainImage,
        dest_size: PixelSize,
        frame: &MappedFrame,
        sprites: &[LayerSprite],
    ) {
        // SAFETY: plain COM call with a live fence.
        unsafe { self.ctx.Wait(frame.fence, frame.fence_in) }.expect("Wait");
        let batch = &mut self.sprite_batch;
        batch.begin(&dest.rtv, dest_size);
        batch.clear([0.0; 4]);

        let tint = frame.config.tint;
        for sprite in sprites {
            let layer_tint = tint.map(|c| c * sprite.opacity);
            batch.draw(
                frame.srv,
                &sprite.source_rect,
                &sprite.dest_rect,
                layer_tint,
            );
        }
        batch.end();
    }
}
