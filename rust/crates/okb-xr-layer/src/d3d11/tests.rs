//! D3D11 boundary tests on a WARP device.
#![allow(unsafe_code)]

use okb_shm::Frame;
use okb_shm::wire::{Config, Origin, PixelPoint, PixelRect, PixelSize};
use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL, HANDLE, HMODULE};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_WARP;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIResource1;
use windows::core::{Interface, PCWSTR};

use super::sprite_batch::VS_DXBC;
use super::*;
use crate::compose::LayerSprite;

type Rgba = [u8; 4];

pub(crate) fn warp() -> (ID3D11Device, ID3D11DeviceContext) {
    let (mut device, mut ctx) = (None, None);
    // SAFETY: out-pointers are valid locals.
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_WARP,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_FLAG(0),
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut ctx),
        )
    }
    .expect("WARP device");
    (device.unwrap(), ctx.unwrap())
}

fn texture(
    device: &ID3D11Device,
    size: u32,
    bind: D3D11_BIND_FLAG,
    misc: D3D11_RESOURCE_MISC_FLAG,
    pixels: Option<&[Rgba]>,
) -> ID3D11Texture2D {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: size,
        Height: size,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: bind.0 as u32,
        MiscFlags: misc.0 as u32,
        ..Default::default()
    };
    let init = pixels.map(|p| D3D11_SUBRESOURCE_DATA {
        pSysMem: p.as_ptr().cast(),
        SysMemPitch: size * 4,
        SysMemSlicePitch: 0,
    });
    let mut tex = None;
    // SAFETY: `desc` and `init` (when present) outlive the call; `pixels` holds size² texels.
    unsafe { device.CreateTexture2D(&desc, init.as_ref().map(|i| i as *const _), Some(&mut tex)) }
        .expect("CreateTexture2D");
    tex.unwrap()
}

fn srv(device: &ID3D11Device, tex: &ID3D11Texture2D) -> ID3D11ShaderResourceView {
    let mut srv = None;
    // SAFETY: valid texture and out-pointer.
    unsafe { device.CreateShaderResourceView(tex, None, Some(&mut srv)) }.expect("SRV");
    srv.unwrap()
}

fn fence(device: &ID3D11Device, flags: D3D11_FENCE_FLAG) -> ID3D11Fence {
    let device5: ID3D11Device5 = device.cast().expect("ID3D11Device5");
    let mut fence: Option<ID3D11Fence> = None;
    // SAFETY: valid out-pointer.
    unsafe { device5.CreateFence(0, flags, &mut fence) }.expect("CreateFence");
    fence.unwrap()
}

fn readback(device: &ID3D11Device, ctx: &ID3D11DeviceContext, tex: &ID3D11Texture2D) -> Vec<Rgba> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: valid out-pointer.
    unsafe { tex.GetDesc(&mut desc) };
    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    desc.MiscFlags = 0;
    let mut staging = None;
    // SAFETY: valid descriptor and out-pointer.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut staging)) }.expect("staging");
    let staging = staging.unwrap();
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    let mut out = Vec::new();
    // SAFETY: the mapping covers `Height` rows of `RowPitch` bytes, each holding
    // `Width` RGBA8 texels; it is read before `Unmap`.
    unsafe {
        ctx.CopyResource(&staging, tex);
        ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .expect("Map");
        for y in 0..desc.Height as usize {
            let row = mapped.pData.cast::<u8>().add(y * mapped.RowPitch as usize);
            out.extend_from_slice(std::slice::from_raw_parts(
                row.cast::<Rgba>(),
                desc.Width as usize,
            ));
        }
        ctx.Unmap(&staging, 0);
    }
    out
}

fn rect(x: u32, y: u32, width: u32, height: u32) -> PixelRect {
    PixelRect {
        offset: PixelPoint { x, y },
        size: PixelSize { width, height },
        origin: Origin::TOP_LEFT,
    }
}

fn target(device: &ID3D11Device) -> SwapchainImage {
    let texture = texture(
        device,
        8,
        D3D11_BIND_RENDER_TARGET,
        D3D11_RESOURCE_MISC_FLAG(0),
        None,
    );
    let mut rtv = None;
    // SAFETY: valid texture and out-pointer.
    unsafe { device.CreateRenderTargetView(&texture, None, Some(&mut rtv)) }.expect("RTV");
    SwapchainImage {
        texture,
        rtv: rtv.unwrap(),
    }
}

/// Renders `sprites` from a 4×4 `source` into a cleared 8×8 target; returns its texels.
fn render(source: &[Rgba; 16], sprites: &[LayerSprite]) -> Vec<Rgba> {
    let (device, ctx) = warp();
    let src = texture(
        &device,
        4,
        D3D11_BIND_SHADER_RESOURCE,
        D3D11_RESOURCE_MISC_FLAG(0),
        Some(source),
    );
    let srv = srv(&device, &src);
    let fence = fence(&device, D3D11_FENCE_FLAG_NONE);
    let config = Config {
        tint: [1.0; 4],
        ..Config::default()
    };
    let frame = MappedFrame {
        config: &config,
        srv: &srv,
        fence: &fence,
        fence_in: 0,
    };
    let dest = target(&device);
    // Pre-fill, so the test proves the renderer clears.
    // SAFETY: valid RTV.
    unsafe { ctx.ClearRenderTargetView(&dest.rtv, &[1.0; 4]) };
    let mut renderer = Renderer::new(&device);
    renderer.render_layers(
        &dest,
        PixelSize {
            width: 8,
            height: 8,
        },
        &frame,
        sprites,
    );
    readback(&device, &ctx, &dest.texture)
}

fn assert_near(actual: Rgba, expected: Rgba, at: (usize, usize)) {
    assert!(
        actual
            .iter()
            .zip(expected)
            .all(|(&a, e)| a.abs_diff(e) <= 1),
        "at {at:?}: {actual:?} != {expected:?} (±1)"
    );
}

#[test]
fn sprite_copies_source_rect_to_dest_rect() {
    let source: [Rgba; 16] =
        std::array::from_fn(|i| [i as u8 * 16, 255 - i as u8 * 8, 7 * i as u8, 255]);
    let out = render(
        &source,
        &[LayerSprite {
            source_rect: rect(0, 0, 4, 4),
            dest_rect: rect(2, 2, 4, 4),
            opacity: 1.0,
        }],
    );
    for y in 0..8 {
        for x in 0..8 {
            let expected = if (2..6).contains(&x) && (2..6).contains(&y) {
                source[(y - 2) * 4 + (x - 2)]
            } else {
                [0; 4]
            };
            assert_eq!(out[y * 8 + x], expected, "at ({x}, {y})");
        }
    }
}

#[test]
fn opacity_and_tint_scale_premultiplied_color() {
    let out = render(
        &[[200, 100, 50, 255]; 16],
        &[LayerSprite {
            source_rect: rect(0, 0, 4, 4),
            dest_rect: rect(0, 0, 4, 4),
            opacity: 0.5,
        }],
    );
    for y in 0..4 {
        for x in 0..4 {
            assert_near(out[y * 8 + x], [100, 50, 25, 128], (x, y));
        }
    }
}

#[test]
fn blend_is_premultiplied_over() {
    const RED: Rgba = [255, 0, 0, 255];
    const HALF_GREEN: Rgba = [0, 128, 0, 128];
    // Left half opaque red, right half premultiplied half-alpha green.
    let source: [Rgba; 16] = std::array::from_fn(|i| if i % 4 < 2 { RED } else { HALF_GREEN });
    let out = render(
        &source,
        &[
            LayerSprite {
                source_rect: rect(0, 0, 2, 4),
                dest_rect: rect(0, 0, 2, 4),
                opacity: 1.0,
            },
            LayerSprite {
                source_rect: rect(2, 0, 2, 4),
                dest_rect: rect(1, 0, 2, 4),
                opacity: 1.0,
            },
        ],
    );
    for y in 0..4 {
        assert_near(out[y * 8], RED, (0, y));
        // green + (1 - 128/255) * red
        assert_near(out[y * 8 + 1], [127, 128, 0, 255], (1, y));
        assert_near(out[y * 8 + 2], HALF_GREEN, (2, y));
        assert_eq!(out[y * 8 + 3], [0; 4]);
    }
}

struct Shared {
    _texture: ID3D11Texture2D,
    _fence: ID3D11Fence,
    texture: HANDLE,
    fence: HANDLE,
}

impl Drop for Shared {
    fn drop(&mut self) {
        // SAFETY: both handles came from `CreateSharedHandle` and are closed once.
        unsafe {
            CloseHandle(self.texture).unwrap();
            CloseHandle(self.fence).unwrap();
        }
    }
}

/// A feeder-style texture and fence, shared through NT handles.
fn shared(device: &ID3D11Device) -> Shared {
    let tex = texture(
        device,
        4,
        D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET,
        D3D11_RESOURCE_MISC_SHARED_NTHANDLE | D3D11_RESOURCE_MISC_SHARED,
        None,
    );
    let fence = fence(device, D3D11_FENCE_FLAG_SHARED);
    // SAFETY: both objects were created shareable; null attributes and name are allowed.
    let (texture, fence_handle) = unsafe {
        (
            tex.cast::<IDXGIResource1>()
                .unwrap()
                .CreateSharedHandle(None, GENERIC_ALL.0, PCWSTR::null())
                .expect("shared texture handle"),
            fence
                .CreateSharedHandle(None, GENERIC_ALL.0, PCWSTR::null())
                .expect("shared fence handle"),
        )
    };
    Shared {
        _texture: tex,
        _fence: fence,
        texture,
        fence: fence_handle,
    }
}

fn frame(shared: &Shared) -> Frame {
    Frame {
        config: Config::default(),
        layers: Vec::new(),
        texture: shared.texture.0 as usize as u64,
        fence: shared.fence.0 as usize as u64,
        fence_in: 0,
        index: 0,
    }
}

#[test]
fn map_reuses_resources_until_handle_changes() {
    let (device, _ctx) = warp();
    let mut client = ShmClient::new(&device).expect("SHM reader");
    let (a, b) = (shared(&device), shared(&device));
    let first = client.map(&frame(&a)).srv.clone();
    assert_eq!(client.map(&frame(&a)).srv, &first);
    let mut other = frame(&b);
    other.fence = frame(&a).fence;
    assert_ne!(client.map(&other).srv, &first);
}

#[test]
fn session_change_drops_cached_resources() {
    let (device, _ctx) = warp();
    let mut client = ShmClient::new(&device).expect("SHM reader");
    let a = shared(&device);
    let first = client.map(&frame(&a)).srv.clone();
    client.session_changed.set(true);
    assert_ne!(client.map(&frame(&a)).srv, &first);
}

#[test]
fn context_state_is_restored() {
    let (device, ctx) = warp();
    let mut vs = None;
    // SAFETY: valid bytecode and out-pointer.
    unsafe { device.CreateVertexShader(VS_DXBC, None, Some(&mut vs)) }.expect("VS");
    let vs = vs.unwrap();
    // SAFETY: valid shader.
    unsafe { ctx.VSSetShader(&vs, None) };

    let ctx1: ID3D11DeviceContext1 = ctx.cast().unwrap();
    let mut state = DeviceContextState::default();
    {
        let _scope = state.scoped(&ctx1);
        let src = texture(
            &device,
            4,
            D3D11_BIND_SHADER_RESOURCE,
            D3D11_RESOURCE_MISC_FLAG(0),
            None,
        );
        let (srv, fence) = (srv(&device, &src), fence(&device, D3D11_FENCE_FLAG_NONE));
        let config = Config::default();
        Renderer::new(&device).render_layers(
            &target(&device),
            PixelSize {
                width: 8,
                height: 8,
            },
            &MappedFrame {
                config: &config,
                srv: &srv,
                fence: &fence,
                fence_in: 0,
            },
            &[LayerSprite {
                source_rect: rect(0, 0, 4, 4),
                dest_rect: rect(0, 0, 4, 4),
                opacity: 1.0,
            }],
        );
    }

    let mut bound = None;
    // SAFETY: valid out-pointer.
    unsafe { ctx.VSGetShader(&mut bound, None, None) };
    assert_eq!(bound.as_ref(), Some(&vs));
}
