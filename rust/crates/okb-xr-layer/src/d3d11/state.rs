//! C++ `D3D11::DeviceContextState` and `ScopedDeviceContextStateChange`
//! (`src/lib/D3D11.cpp`).
#![allow(unsafe_code)]

use windows::Win32::Graphics::Direct3D11::{
    D3D11_1_CREATE_DEVICE_CONTEXT_STATE_SINGLETHREADED, D3D11_CREATE_DEVICE_SINGLETHREADED,
    D3D11_SDK_VERSION, ID3D11Device, ID3D11Device1, ID3D11DeviceContext1, ID3DDeviceContextState,
};
use windows::core::Interface;

/// A private device-context state, created on first use.
#[derive(Default)]
pub struct DeviceContextState(Option<ID3DDeviceContextState>);

/// Swaps the original state back in on drop.
pub struct ScopedState<'a> {
    ctx: &'a ID3D11DeviceContext1,
    original: Option<ID3DDeviceContextState>,
}

impl DeviceContextState {
    /// C++ `DeviceContextState(ID3D11Device1*)`.
    fn create(device: &ID3D11Device1) -> ID3DDeviceContextState {
        let mut state = None;
        // SAFETY: valid feature-level slice, IID and out-pointer.
        unsafe {
            let flags = if device.GetCreationFlags() & D3D11_CREATE_DEVICE_SINGLETHREADED.0 != 0 {
                D3D11_1_CREATE_DEVICE_CONTEXT_STATE_SINGLETHREADED.0 as u32
            } else {
                0
            };
            device.CreateDeviceContextState(
                flags,
                &[device.GetFeatureLevel()],
                D3D11_SDK_VERSION,
                &ID3D11Device::IID,
                None,
                Some(&mut state),
            )
        }
        .expect("CreateDeviceContextState");
        state.expect("CreateDeviceContextState returned null")
    }

    /// C++ `ScopedDeviceContextStateChange`: makes this state current until the
    /// guard drops.
    ///
    /// # Panics
    /// On HRESULT failure.
    pub fn scoped<'a>(&'a mut self, ctx: &'a ID3D11DeviceContext1) -> ScopedState<'a> {
        let state = self.0.get_or_insert_with(|| {
            // SAFETY: plain COM call on a live context.
            let device = unsafe { ctx.GetDevice() }.expect("GetDevice");
            Self::create(&device.cast().expect("ID3D11Device1"))
        });
        let mut original = None;
        // SAFETY: valid state and out-pointer.
        unsafe { ctx.SwapDeviceContextState(&*state, Some(&mut original)) };
        ScopedState { ctx, original }
    }
}

impl Drop for ScopedState<'_> {
    fn drop(&mut self) {
        // SAFETY: `original` came from `SwapDeviceContextState` on this context.
        unsafe {
            self.ctx
                .SwapDeviceContextState(self.original.as_ref(), None)
        };
    }
}
