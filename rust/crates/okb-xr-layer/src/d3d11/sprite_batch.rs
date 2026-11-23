//! C++ `D3D11::SpriteBatch` (`src/lib/D3D11/SpriteBatch.cpp`), with the
//! DirectXTK `CommonStates` it uses.
#![allow(unsafe_code)]

use okb_shm::MAX_VIEW_COUNT;
use okb_shm::wire::{Origin, PixelRect, PixelSize};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R32G32B32A32_FLOAT,
};
use windows::core::{Interface, s};

pub(super) const VS_DXBC: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite_vs.dxbc"));
const PS_DXBC: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite_ps.dxbc"));

/// C++ `ShaderData::Uniform`.
#[repr(C)]
struct Uniform {
    source_dimensions: [f32; 2],
    dest_dimensions: [f32; 2],
}

/// C++ `ShaderData::Vertex`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 4],
    color: [f32; 4],
    tex_coord: [f32; 2],
    tex_clamp_tl: [f32; 2],
    tex_clamp_br: [f32; 2],
}

/// A rectangle is two triangles of three vertices.
const MAX_VERTICES: usize = 6 * MAX_VIEW_COUNT;

/// C++ `Rect<uint32_t>::Bottom()`; wraps for bottom-left origins, as unsigned C++ does.
fn bottom(r: &PixelRect) -> u32 {
    if r.origin == Origin::TOP_LEFT {
        r.offset.y.wrapping_add(r.size.height)
    } else {
        r.offset.y.wrapping_sub(r.size.height)
    }
}

/// C++ `Rect<uint32_t>::Bottom<float>()`.
fn bottom_f(r: &PixelRect) -> f32 {
    if r.origin == Origin::TOP_LEFT {
        r.offset.y as f32 + r.size.height as f32
    } else {
        r.offset.y as f32 - r.size.height as f32
    }
}

/// C++ `Rect<uint32_t>::Right()`.
fn right(r: &PixelRect) -> u32 {
    r.offset.x.wrapping_add(r.size.width)
}

pub(super) struct SpriteBatch {
    ctx: ID3D11DeviceContext,
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    uniform_buffer: ID3D11Buffer,
    input_layout: ID3D11InputLayout,
    vertex_buffer: ID3D11Buffer,
    linear_clamp: ID3D11SamplerState,
    cull_counter_clockwise: ID3D11RasterizerState,
    depth_none: ID3D11DepthStencilState,
    alpha_blend: ID3D11BlendState,

    target: Option<ID3D11RenderTargetView>,
    target_dimensions: [f32; 2],
    pending_source: Option<ID3D11ShaderResourceView>,
    pending_source_desc: D3D11_TEXTURE2D_DESC,
    pending_vertices: Vec<Vertex>,
}

/// Unwraps a COM out-parameter after a successful create call.
fn created<T>(hr: windows::core::Result<()>, out: Option<T>, what: &str) -> T {
    hr.unwrap_or_else(|e| panic!("{what}: {e}"));
    out.unwrap_or_else(|| panic!("{what} returned null"))
}

fn buffer(device: &ID3D11Device, byte_width: usize, bind: D3D11_BIND_FLAG) -> ID3D11Buffer {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: byte_width as u32,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: bind.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        ..Default::default()
    };
    let mut out = None;
    // SAFETY: valid descriptor and out-pointer.
    let hr = unsafe { device.CreateBuffer(&desc, None, Some(&mut out)) };
    created(hr, out, "CreateBuffer")
}

impl SpriteBatch {
    /// # Panics
    /// On HRESULT failure.
    pub(super) fn new(device: &ID3D11Device) -> Self {
        // SAFETY: plain COM call on a live device.
        let ctx = unsafe { device.GetImmediateContext() }.expect("GetImmediateContext");

        let (mut ps, mut vs) = (None, None);
        // SAFETY: valid fxc bytecode and out-pointers.
        let (ps_hr, vs_hr) = unsafe {
            (
                device.CreatePixelShader(PS_DXBC, None, Some(&mut ps)),
                device.CreateVertexShader(VS_DXBC, None, Some(&mut vs)),
            )
        };
        let pixel_shader = created(ps_hr, ps, "CreatePixelShader");
        let vertex_shader = created(vs_hr, vs, "CreateVertexShader");

        let uniform_buffer = buffer(device, size_of::<Uniform>(), D3D11_BIND_CONSTANT_BUFFER);

        let element = |name, index, format, offset| D3D11_INPUT_ELEMENT_DESC {
            SemanticName: name,
            SemanticIndex: index,
            Format: format,
            InputSlot: 0,
            AlignedByteOffset: offset,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        };
        let elements = [
            element(s!("SV_Position"), 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 0),
            element(
                s!("COLOR"),
                0,
                DXGI_FORMAT_R32G32B32A32_FLOAT,
                D3D11_APPEND_ALIGNED_ELEMENT,
            ),
            element(
                s!("TEXCOORD"),
                0,
                DXGI_FORMAT_R32G32_FLOAT,
                D3D11_APPEND_ALIGNED_ELEMENT,
            ),
            element(
                s!("TEXCOORD"),
                1,
                DXGI_FORMAT_R32G32_FLOAT,
                D3D11_APPEND_ALIGNED_ELEMENT,
            ),
            element(
                s!("TEXCOORD"),
                2,
                DXGI_FORMAT_R32G32_FLOAT,
                D3D11_APPEND_ALIGNED_ELEMENT,
            ),
        ];
        let mut layout = None;
        // SAFETY: the semantic names are static C strings; valid out-pointer.
        let hr = unsafe { device.CreateInputLayout(&elements, VS_DXBC, Some(&mut layout)) };
        let input_layout = created(hr, layout, "CreateInputLayout");

        let vertex_buffer = buffer(
            device,
            size_of::<Vertex>() * MAX_VERTICES,
            D3D11_BIND_VERTEX_BUFFER,
        );

        // DirectXTK `CommonStates::LinearClamp()`.
        let sampler = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
            MaxAnisotropy: D3D11_MAX_MAXANISOTROPY,
            ComparisonFunc: D3D11_COMPARISON_NEVER,
            MaxLOD: f32::MAX,
            ..Default::default()
        };
        // DirectXTK `CommonStates::CullCounterClockwise()`.
        let rasterizer = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_BACK,
            DepthClipEnable: true.into(),
            MultisampleEnable: true.into(),
            ..Default::default()
        };
        // DirectXTK `CommonStates::DepthNone()`.
        let stencil_op = D3D11_DEPTH_STENCILOP_DESC {
            StencilFailOp: D3D11_STENCIL_OP_KEEP,
            StencilDepthFailOp: D3D11_STENCIL_OP_KEEP,
            StencilPassOp: D3D11_STENCIL_OP_KEEP,
            StencilFunc: D3D11_COMPARISON_ALWAYS,
        };
        let depth = D3D11_DEPTH_STENCIL_DESC {
            DepthEnable: false.into(),
            DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ZERO,
            DepthFunc: D3D11_COMPARISON_LESS_EQUAL,
            StencilEnable: false.into(),
            StencilReadMask: D3D11_DEFAULT_STENCIL_READ_MASK as u8,
            StencilWriteMask: D3D11_DEFAULT_STENCIL_WRITE_MASK as u8,
            FrontFace: stencil_op,
            BackFace: stencil_op,
        };
        // DirectXTK `CommonStates::AlphaBlend()`: premultiplied over.
        let mut blend = D3D11_BLEND_DESC::default();
        blend.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: true.into(),
            SrcBlend: D3D11_BLEND_ONE,
            DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_ONE,
            DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
        };

        let (mut s, mut r, mut d, mut b) = (None, None, None, None);
        // SAFETY: valid descriptors and out-pointers.
        let (s_hr, r_hr, d_hr, b_hr) = unsafe {
            (
                device.CreateSamplerState(&sampler, Some(&mut s)),
                device.CreateRasterizerState(&rasterizer, Some(&mut r)),
                device.CreateDepthStencilState(&depth, Some(&mut d)),
                device.CreateBlendState(&blend, Some(&mut b)),
            )
        };

        Self {
            ctx,
            vertex_shader,
            pixel_shader,
            uniform_buffer,
            input_layout,
            vertex_buffer,
            linear_clamp: created(s_hr, s, "CreateSamplerState"),
            cull_counter_clockwise: created(r_hr, r, "CreateRasterizerState"),
            depth_none: created(d_hr, d, "CreateDepthStencilState"),
            alpha_blend: created(b_hr, b, "CreateBlendState"),
            target: None,
            target_dimensions: [0.0; 2],
            pending_source: None,
            pending_source_desc: D3D11_TEXTURE2D_DESC::default(),
            pending_vertices: Vec::with_capacity(MAX_VERTICES),
        }
    }

    /// Binds the whole pipeline state; the caller can't assume anything about it.
    ///
    /// # Panics
    /// If a frame is already in progress (C++ `fatal()`).
    pub(super) fn begin(&mut self, rtv: &ID3D11RenderTargetView, size: PixelSize) {
        assert!(
            self.target.is_none(),
            "frame already in progress; did you call End()?"
        );
        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: size.width as f32,
            Height: size.height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        let scissor = RECT {
            left: 0,
            top: 0,
            right: size.width as i32,
            bottom: size.height as i32,
        };
        let uniform_buffers = [Some(self.uniform_buffer.clone())];
        let vertex_buffers = [Some(self.vertex_buffer.clone())];
        let stride = size_of::<Vertex>() as u32;
        let offset = 0u32;

        let ctx = &self.ctx;
        // SAFETY: every pointer and slice is a live local or field for the call.
        unsafe {
            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.IASetInputLayout(&self.input_layout);
            ctx.IASetVertexBuffers(
                0,
                1,
                Some(vertex_buffers.as_ptr()),
                Some(&stride),
                Some(&offset),
            );

            ctx.VSSetConstantBuffers(0, Some(&uniform_buffers));
            ctx.VSSetShader(&self.vertex_shader, None);

            ctx.PSSetConstantBuffers(0, Some(&uniform_buffers));
            ctx.PSSetShader(&self.pixel_shader, None);
            ctx.PSSetShaderResources(0, Some(&[None]));
            ctx.PSSetSamplers(0, Some(&[Some(self.linear_clamp.clone())]));

            ctx.RSSetState(&self.cull_counter_clockwise);
            ctx.RSSetViewports(Some(&[viewport]));
            ctx.RSSetScissorRects(Some(&[scissor]));

            ctx.OMSetRenderTargets(Some(&[Some(rtv.clone())]), None);
            ctx.OMSetDepthStencilState(&self.depth_none, 0);
            ctx.OMSetBlendState(&self.alpha_blend, Some(&[1.0; 4]), u32::MAX);
        }

        self.target = Some(rtv.clone());
        self.target_dimensions = [size.width as f32, size.height as f32];
    }

    /// # Panics
    /// Outside `begin`/`end` (C++ `fatal()`).
    pub(super) fn clear(&mut self, color: [f32; 4]) {
        let target = self
            .target
            .as_ref()
            .expect("target not set, call BeginFrame()");
        // SAFETY: valid RTV and color.
        unsafe { self.ctx.ClearRenderTargetView(target, &color) };
    }

    /// # Panics
    /// Outside `begin`/`end` (C++ `fatal()`), or on HRESULT failure.
    pub(super) fn draw(
        &mut self,
        source: &ID3D11ShaderResourceView,
        source_rect: &PixelRect,
        dest_rect: &PixelRect,
        tint: [f32; 4],
    ) {
        assert!(self.target.is_some(), "target not set, call BeginFrame()");

        if self.pending_source.as_ref() != Some(source) {
            self.draw_pending_vertices();
            self.pending_source = Some(source.clone());
            // SAFETY: plain COM calls on a live view; valid out-pointer.
            unsafe {
                let texture: ID3D11Texture2D = source
                    .GetResource()
                    .and_then(|r| r.cast())
                    .expect("source is not a Texture2D");
                texture.GetDesc(&mut self.pending_source_desc);
            }
        }
        let desc = &self.pending_source_desc;
        let (width, height) = (desc.Width as f32, desc.Height as f32);

        let tex_clamp_tl = [
            (source_rect.offset.x as f32 + 0.5) / width,
            (source_rect.offset.y as f32 + 0.5) / height,
        ];
        let tex_clamp_br = [
            (right(source_rect) as f32 - 0.5) / width,
            (bottom(source_rect) as f32 - 0.5) / height,
        ];

        let src_tl = [source_rect.offset.x as f32, source_rect.offset.y as f32];
        let src_br = [right(source_rect) as f32, bottom(source_rect) as f32];
        let src_bl = [src_tl[0], src_br[1]];
        let src_tr = [src_br[0], src_tl[1]];

        let (left, top) = (dest_rect.offset.x as f32, dest_rect.offset.y as f32);
        let (right, bottom) = (left + dest_rect.size.width as f32, bottom_f(dest_rect));
        let dst_tl = [left, top, 0.0, 1.0];
        let dst_br = [right, bottom, 0.0, 1.0];
        let dst_bl = [left, bottom, 0.0, 1.0];
        let dst_tr = [right, top, 0.0, 1.0];

        let vertex = |tex_coord, position| Vertex {
            position,
            color: tint,
            tex_coord,
            tex_clamp_tl,
            tex_clamp_br,
        };
        self.pending_vertices.extend([
            // First triangle: excludes top right
            vertex(src_bl, dst_bl),
            vertex(src_tl, dst_tl),
            vertex(src_br, dst_br),
            // Second triangle: excludes bottom left
            vertex(src_tl, dst_tl),
            vertex(src_tr, dst_tr),
            vertex(src_br, dst_br),
        ]);
    }

    /// # Panics
    /// Without a matching `begin` (C++ `fatal()`).
    pub(super) fn end(&mut self) {
        assert!(
            self.target.is_some(),
            "target not set; double-End() or Begin() not called?"
        );
        self.draw_pending_vertices();
        // SAFETY: a one-element slice of a null RTV.
        unsafe { self.ctx.OMSetRenderTargets(Some(&[None]), None) };
        self.target = None;
    }

    fn draw_pending_vertices(&mut self) {
        if self.pending_vertices.is_empty() {
            return;
        }
        // C++ `memcpy`s past the buffer here (UB); Rust panics instead.
        assert!(
            self.pending_vertices.len() <= MAX_VERTICES,
            "{} vertices exceed the {MAX_VERTICES}-vertex buffer",
            self.pending_vertices.len()
        );
        let uniform = Uniform {
            source_dimensions: [
                self.pending_source_desc.Width as f32,
                self.pending_source_desc.Height as f32,
            ],
            dest_dimensions: self.target_dimensions,
        };
        let ctx = &self.ctx;
        // SAFETY: each mapping is at least as large as what is written (the
        // uniform buffer is `size_of::<Uniform>()`, the vertex buffer holds
        // `MAX_VERTICES`), and is written before `Unmap`.
        unsafe {
            let mut mapping = D3D11_MAPPED_SUBRESOURCE::default();
            ctx.Map(
                &self.uniform_buffer,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                Some(&mut mapping),
            )
            .expect("Map uniform buffer");
            mapping.pData.cast::<Uniform>().write(uniform);
            ctx.Unmap(&self.uniform_buffer, 0);

            ctx.Map(
                &self.vertex_buffer,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                Some(&mut mapping),
            )
            .expect("Map vertex buffer");
            std::ptr::copy_nonoverlapping(
                self.pending_vertices.as_ptr(),
                mapping.pData.cast::<Vertex>(),
                self.pending_vertices.len(),
            );
            ctx.Unmap(&self.vertex_buffer, 0);

            ctx.PSSetShaderResources(0, Some(&[self.pending_source.take()]));
            ctx.Draw(self.pending_vertices.len() as u32, 0);
        }
        self.pending_vertices.clear();
    }
}
