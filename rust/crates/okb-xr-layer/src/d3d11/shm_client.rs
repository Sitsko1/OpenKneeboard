//! C++ `SHM::D3D11::Reader` (`src/lib/SHM/D3D11.cpp`).
#![allow(unsafe_code)]

use std::cell::Cell;
use std::rc::Rc;

use okb_shm::wire::Config;
use okb_shm::{ConsumerKind, Frame, Reader, SWAPCHAIN_LENGTH};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11Device5, ID3D11Fence, ID3D11ShaderResourceView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::core::Interface;

/// C++ `SHM::D3D11::Frame`; `layers` stay in the [`Frame`].
pub struct MappedFrame<'a> {
    pub config: &'a Config,
    pub srv: &'a ID3D11ShaderResourceView,
    pub fence: &'a ID3D11Fence,
    pub fence_in: u64,
}

/// C++ `FrameD3D11Resources`.
#[derive(Default)]
struct CachedResources {
    texture_handle: u64,
    texture: Option<ID3D11Texture2D>,
    srv: Option<ID3D11ShaderResourceView>,
    fence_handle: u64,
    fence: Option<ID3D11Fence>,
}

pub struct ShmClient {
    reader: Reader<Box<dyn FnMut()>>,
    /// Set by the reader's session callback; [`ShmClient::map`] then drops the
    /// cache (C++ `OnSessionChanged`: `mFrames = {}`).
    pub(super) session_changed: Rc<Cell<bool>>,
    device: ID3D11Device5,
    frames: [CachedResources; SWAPCHAIN_LENGTH],
}

/// The adapter LUID, bit-cast to `u64` as C++ `GetLUID` does.
///
/// # Panics
/// On HRESULT failure (C++ `check_hresult`).
fn luid(device: &ID3D11Device) -> u64 {
    let dxgi: IDXGIDevice = device.cast().expect("IDXGIDevice");
    // SAFETY: plain COM calls on live interfaces.
    let desc = unsafe { dxgi.GetAdapter().and_then(|a| a.GetDesc()) }.expect("adapter desc");
    let luid = desc.AdapterLuid;
    (u64::from(luid.HighPart as u32) << 32) | u64::from(luid.LowPart)
}

impl ShmClient {
    /// `None` where the SHM reader can't be opened.
    ///
    /// # Panics
    /// If the device lacks `ID3D11Device5` or the LUID query fails (C++ throws).
    pub fn new(device: &ID3D11Device) -> Option<Self> {
        let session_changed = Rc::new(Cell::new(false));
        let flag = session_changed.clone();
        let reader = Reader::new(
            &crate::identity::shm_names(),
            ConsumerKind::OpenXrD3d11,
            luid(device),
            Box::new(move || flag.set(true)) as Box<dyn FnMut()>,
        )?;
        Some(Self {
            reader,
            session_changed,
            device: device.cast().expect("ID3D11Device5"),
            frames: Default::default(),
        })
    }

    pub fn reader(&mut self) -> &mut Reader<Box<dyn FnMut()>> {
        &mut self.reader
    }

    /// C++ `SHM::D3D11::Reader::Map`.
    ///
    /// # Panics
    /// On HRESULT failure, or an `index` past the swapchain (C++ `.at()`).
    pub fn map<'a>(&'a mut self, frame: &'a Frame) -> MappedFrame<'a> {
        if self.session_changed.replace(false) {
            self.frames = Default::default();
        }
        let com = &mut self.frames[usize::from(frame.index)];
        if com.texture_handle != frame.texture {
            com.texture_handle = frame.texture;
            com.texture = None;
            com.srv = None;
        }
        if com.fence_handle != frame.fence {
            com.fence_handle = frame.fence;
            com.fence = None;
        }

        let device = &self.device;
        let (texture_handle, fence_handle) = (handle(com.texture_handle), handle(com.fence_handle));
        let texture = com.texture.get_or_insert_with(|| {
            // SAFETY: the reader owns the handle until the next session change,
            // which also clears this cache before it is used again.
            unsafe { device.OpenSharedResource1(texture_handle) }.expect("OpenSharedResource1")
        });
        let srv = com.srv.get_or_insert_with(|| {
            let mut srv = None;
            // SAFETY: valid texture and out-pointer.
            unsafe { device.CreateShaderResourceView(&*texture, None, Some(&mut srv)) }
                .expect("CreateShaderResourceView");
            srv.expect("CreateShaderResourceView returned null")
        });
        let fence = com.fence.get_or_insert_with(|| {
            let mut fence = None;
            // SAFETY: as for the texture handle; valid out-pointer.
            unsafe { device.OpenSharedFence(fence_handle, &mut fence) }.expect("OpenSharedFence");
            fence.expect("OpenSharedFence returned null")
        });

        MappedFrame {
            config: &frame.config,
            srv,
            fence,
            fence_in: frame.fence_in as u64,
        }
    }
}

fn handle(raw: u64) -> HANDLE {
    HANDLE(raw as usize as *mut _)
}
