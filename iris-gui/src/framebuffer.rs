//! Headless renderer that captures REX3's framebuffer into a shared buffer
//! for egui to upload as a texture each frame.

use iris::rex3::Renderer;
use parking_lot::{Mutex, MutexGuard};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// One captured frame: tightly-packed RGBA bytes plus pixel dimensions.
#[derive(Clone, Default)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    /// Length is width * height * 4. Pixel order: R, G, B, A.
    pub rgba: Vec<u8>,
    /// Bumped every time `Renderer::render` lands a new buffer; egui uses
    /// this to skip the texture upload when nothing has changed.
    pub seq: u64,
}

/// Shared latest-frame slot. The renderer writes; the GUI reads.
///
/// `seq` is mirrored into a lock-free atomic so the GUI can ask "is there a
/// new frame?" on every repaint without taking the mutex or cloning the
/// multi-MB buffer. It only locks + clones when the sequence actually moved.
#[derive(Default, Clone)]
pub struct FrameSink {
    frame: Arc<Mutex<Frame>>,
    seq: Arc<AtomicU64>,
}

impl FrameSink {
    pub fn new() -> Self { Self::default() }

    /// Lock-free latest sequence number (0 = no frame produced yet).
    pub fn seq(&self) -> u64 { self.seq.load(Ordering::Acquire) }

    /// Clone the latest frame out. Callers should gate this on `seq()` having
    /// changed so they don't copy the whole buffer when nothing is new.
    pub fn snapshot(&self) -> Frame { self.frame.lock().clone() }

    fn lock(&self) -> MutexGuard<'_, Frame> { self.frame.lock() }
}

/// `iris::rex3::Renderer` implementation that copies each `render` call
/// into a `FrameSink`. The REX3 refresh thread invokes us at video rate;
/// we keep the work minimal (one pass) so we don't stall it.
pub struct CaptureRenderer {
    sink: FrameSink,
    seq:  u64,
}

impl CaptureRenderer {
    pub fn new(sink: FrameSink) -> Self {
        Self { sink, seq: 0 }
    }
}

impl Renderer for CaptureRenderer {
    fn render(&mut self, buffer: &[u32], width: usize, height: usize) {
        // The REX3 framebuffer has row stride 2048 in u32 words regardless
        // of `width` — typical mode is 1280×1024 displayable.
        const STRIDE: usize = 2048;
        if width == 0 || height == 0 { return; }
        let needed = width.checked_mul(height).and_then(|n| n.checked_mul(4)).unwrap_or(0);
        if needed == 0 { return; }

        let mut frame = self.sink.lock();
        if frame.rgba.len() != needed {
            frame.rgba = vec![0u8; needed];
        }
        frame.width = width;
        frame.height = height;

        // REX3 pixel layout is RGBA in u32 little-endian: R is the low byte,
        // matching what egui's `ColorImage::from_rgba_unmultiplied` expects.
        // The high byte is NOT opacity — REX3 packs dither/overlay bits there
        // (e.g. the Bayer index) — but egui composites textures with alpha
        // blending, so we must force alpha to 0xFF or the frame renders
        // (near-)transparent (black). Do the pack-and-opaque in a single pass
        // per row: `word | 0xFF00_0000` writes R,G,B verbatim and A = 0xFF.
        for y in 0..height {
            let src_row_start = y * STRIDE;
            let src_row_end   = src_row_start + width;
            if src_row_end > buffer.len() { break; }
            let src_row = &buffer[src_row_start..src_row_end];
            let dst_row_start = y * width * 4;
            let dst_row_end   = dst_row_start + width * 4;
            let dst_row       = &mut frame.rgba[dst_row_start..dst_row_end];

            for (dst_px, &word) in dst_row.chunks_exact_mut(4).zip(src_row) {
                dst_px.copy_from_slice(&(word | 0xFF00_0000).to_le_bytes());
            }
        }

        self.seq = self.seq.wrapping_add(1);
        frame.seq = self.seq;
        drop(frame);
        // Publish the new sequence after the buffer write is visible.
        self.sink.seq.store(self.seq, Ordering::Release);
    }

    fn resize(&mut self, _width: usize, _height: usize) {
        // No-op: we resize the destination buffer in `render` based on the
        // actual `width × height` of the next frame.
    }
}
