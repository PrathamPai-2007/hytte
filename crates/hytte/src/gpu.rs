//! GPU presentation: a D3D11 device, a Direct2D device context that draws into a
//! premultiplied-alpha composition swap chain, and DirectComposition to show it.
//!
//! Compared with the DIB + `UpdateLayeredWindow` path in `render.rs` this skips the
//! GPU -> CPU readback and the per-frame copy into the compositor. The window must be
//! created with `WS_EX_NOREDIRECTIONBITMAP` (not layered). The swap chain is the whole
//! canvas; the caller trims what is visible and clickable with a window region.

use std::cell::Cell;
use windows::core::Interface;
use windows::Win32::Foundation::{E_FAIL, HMODULE, HWND};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

pub struct Gpu {
    /// Draw target for the renderer; the swap chain's back buffer is attached per frame.
    pub dc: ID2D1DeviceContext,
    swap: IDXGISwapChain1,
    comp: IDCompositionDevice,
    fx: IDCompositionEffectGroup,
    // Kept alive: dropping the target or visual detaches the swap chain from the window.
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    size: Cell<(u32, u32)>,
    opacity: Cell<f32>,
}

impl Gpu {
    pub fn new(factory: &ID2D1Factory1, hwnd: HWND, w: u32, h: u32) -> windows::core::Result<Self> {
        unsafe {
            let mut d3d = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut d3d),
                None,
                None,
            )?;
            let d3d = d3d.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
            let dxgi: IDXGIDevice = d3d.cast()?;
            let dc = factory
                .CreateDevice(&dxgi)?
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            // Grayscale AA: ClearType needs an opaque background.
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);

            let adapter = dxgi.GetAdapter()?;
            let dxgi_factory: IDXGIFactory2 = adapter.GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: w,
                Height: h,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                Flags: 0,
            };
            let swap = dxgi_factory.CreateSwapChainForComposition(&d3d, &desc, None)?;

            let comp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let target = comp.CreateTargetForHwnd(hwnd, true)?;
            let visual = comp.CreateVisual()?;
            visual.SetContent(&swap)?;
            let fx = comp.CreateEffectGroup()?;
            fx.SetOpacity2(1.0)?;
            visual.SetEffect(&fx)?;
            target.SetRoot(&visual)?;
            comp.Commit()?;
            Ok(Self {
                dc,
                swap,
                comp,
                fx,
                _target: target,
                _visual: visual,
                size: Cell::new((w, h)),
                opacity: Cell::new(1.0),
            })
        }
    }

    /// Match the swap chain to a new canvas size in device pixels.
    pub fn resize(&self, w: u32, h: u32) {
        if self.size.get() == (w, h) {
            return;
        }
        unsafe {
            // No back-buffer reference may be alive here: `end` drops the target.
            let _ = self.dc.SetTarget(None);
            if self
                .swap
                .ResizeBuffers(0, w, h, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))
                .is_ok()
            {
                self.size.set((w, h));
            }
        }
    }

    /// Point the device context at the current back buffer.
    pub fn begin(&self) -> bool {
        unsafe {
            let Ok(surface) = self.swap.GetBuffer::<IDXGISurface>(0) else {
                return false;
            };
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                ..Default::default()
            };
            let Ok(bitmap) = self.dc.CreateBitmapFromDxgiSurface(&surface, Some(&props)) else {
                return false;
            };
            self.dc.SetTarget(&bitmap);
            true
        }
    }

    /// Release the back buffer after drawing.
    pub fn end(&self) {
        unsafe { self.dc.SetTarget(None) };
    }

    /// Show the frame. `opacity` fades the whole pill. False when the device was lost
    /// and the caller must rebuild the renderer.
    pub fn present(&self, opacity: f32) -> bool {
        unsafe {
            if (opacity - self.opacity.get()).abs() > 0.001 {
                self.opacity.set(opacity);
                let _ = self.fx.SetOpacity2(opacity);
                let _ = self.comp.Commit();
            }
            let hr = self.swap.Present(0, DXGI_PRESENT(0));
            !(hr == DXGI_ERROR_DEVICE_REMOVED || hr == DXGI_ERROR_DEVICE_RESET)
        }
    }
}
