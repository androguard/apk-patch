//! Nine-patch PNG (`npTc` chunk) helpers — Apktool-style decode to editable bordered PNG.

use std::io::Cursor;

use png::{BitDepth, ColorType, Decoder, Encoder};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum NinePatchError {
    #[error("not a PNG")]
    NotPng,
    #[error("missing npTc chunk")]
    MissingChunk,
    #[error("truncated or invalid npTc")]
    InvalidChunk,
    #[error("png error: {0}")]
    Png(String),
}

/// Decoded Android nine-patch chunk (`npTc`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NinePatchChunk {
    pub was_deserialized: bool,
    pub num_x_divs: u8,
    pub num_y_divs: u8,
    pub num_colors: u8,
    pub x_divs: Vec<i32>,
    pub y_divs: Vec<i32>,
    pub colors: Vec<u32>,
    pub padding_left: i32,
    pub padding_right: i32,
    pub padding_top: i32,
    pub padding_bottom: i32,
}

/// True if path or bytes indicate a `.9.png` nine-patch.
pub fn is_nine_patch_png(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".9.png")
}

/// Locate and parse the `npTc` chunk from a PNG.
pub fn parse_nine_patch(png: &[u8]) -> Result<NinePatchChunk, NinePatchError> {
    if png.len() < 8 || &png[0..8] != b"\x89PNG\r\n\x1a\n" {
        return Err(NinePatchError::NotPng);
    }
    let mut offset = 8usize;
    while offset + 12 <= png.len() {
        let len = u32::from_be_bytes(png[offset..offset + 4].try_into().unwrap()) as usize;
        let ctype = &png[offset + 4..offset + 8];
        let data_start = offset + 8;
        let data_end = data_start + len;
        if data_end + 4 > png.len() {
            break;
        }
        if ctype == b"npTc" {
            return parse_nptc(&png[data_start..data_end]);
        }
        offset = data_end + 4;
    }
    Err(NinePatchError::MissingChunk)
}

/// Decode a compiled nine-patch PNG (with `npTc`) into an editable `.9.png`
/// with a 1px black border marking stretch patches and content padding
/// (Apktool / aapt convention). If there is no `npTc`, returns the input unchanged.
pub fn decode_nine_patch_png(png: &[u8]) -> Result<Vec<u8>, NinePatchError> {
    let chunk = match parse_nine_patch(png) {
        Ok(c) => c,
        Err(NinePatchError::MissingChunk) => return Ok(png.to_vec()),
        Err(e) => return Err(e),
    };

    let decoder = Decoder::new(Cursor::new(png));
    let mut reader = decoder
        .read_info()
        .map_err(|e| NinePatchError::Png(e.to_string()))?;
    let info = reader.info().clone();
    let w = info.width as usize;
    let h = info.height as usize;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let frame = reader
        .next_frame(&mut buf)
        .map_err(|e| NinePatchError::Png(e.to_string()))?;
    let color = frame.color_type;
    let depth = frame.bit_depth;
    if depth != BitDepth::Eight {
        // Leave exotic depths as-is.
        return Ok(png.to_vec());
    }

    let src_stride = match color {
        ColorType::Rgba => 4,
        ColorType::Rgb => 3,
        ColorType::Grayscale => 1,
        ColorType::GrayscaleAlpha => 2,
        _ => return Ok(png.to_vec()),
    };

    // Output always RGBA, size (w+2) x (h+2).
    let ow = w + 2;
    let oh = h + 2;
    let mut out = vec![0u8; ow * oh * 4];

    let set_px = |img: &mut [u8], x: usize, y: usize, rgba: [u8; 4]| {
        if x >= ow || y >= oh {
            return;
        }
        let i = (y * ow + x) * 4;
        img[i..i + 4].copy_from_slice(&rgba);
    };

    // Copy interior pixels (offset +1,+1).
    for y in 0..h {
        for x in 0..w {
            let si = (y * w + x) * src_stride;
            let rgba = match src_stride {
                4 => [buf[si], buf[si + 1], buf[si + 2], buf[si + 3]],
                3 => [buf[si], buf[si + 1], buf[si + 2], 255],
                2 => [buf[si], buf[si], buf[si], buf[si + 1]],
                _ => [buf[si], buf[si], buf[si], 255],
            };
            set_px(&mut out, x + 1, y + 1, rgba);
        }
    }

    let black = [0u8, 0u8, 0u8, 255u8];

    // Top/bottom stretch markers from xDivs (pairs).
    let mut i = 0;
    while i + 1 < chunk.x_divs.len() {
        let start = chunk.x_divs[i].max(0) as usize;
        let end = chunk.x_divs[i + 1].max(0) as usize;
        for x in start..end.min(w) {
            set_px(&mut out, x + 1, 0, black); // top
        }
        i += 2;
    }
    // Left/right stretch markers from yDivs.
    i = 0;
    while i + 1 < chunk.y_divs.len() {
        let start = chunk.y_divs[i].max(0) as usize;
        let end = chunk.y_divs[i + 1].max(0) as usize;
        for y in start..end.min(h) {
            set_px(&mut out, 0, y + 1, black); // left
        }
        i += 2;
    }

    // Padding markers on right / bottom edges.
    let pad_l = chunk.padding_left.max(0) as usize;
    let pad_r = chunk.padding_right.max(0) as usize;
    let pad_t = chunk.padding_top.max(0) as usize;
    let pad_b = chunk.padding_bottom.max(0) as usize;
    let content_right = w.saturating_sub(pad_r);
    let content_bottom = h.saturating_sub(pad_b);
    for x in pad_l..content_right.min(w) {
        set_px(&mut out, x + 1, oh - 1, black); // bottom
    }
    for y in pad_t..content_bottom.min(h) {
        set_px(&mut out, ow - 1, y + 1, black); // right
    }

    let mut encoded = Vec::new();
    {
        let mut encoder = Encoder::new(&mut encoded, ow as u32, oh as u32);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| NinePatchError::Png(e.to_string()))?;
        writer
            .write_image_data(&out)
            .map_err(|e| NinePatchError::Png(e.to_string()))?;
    }
    Ok(encoded)
}

fn parse_nptc(data: &[u8]) -> Result<NinePatchChunk, NinePatchError> {
    if data.len() < 32 {
        return Err(NinePatchError::InvalidChunk);
    }
    let was_deserialized = data[0] != 0;
    let num_x_divs = data[1];
    let num_y_divs = data[2];
    let num_colors = data[3];
    let x_off = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
    let y_off = u32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;
    let padding_left = i32::from_le_bytes(data[12..16].try_into().unwrap());
    let padding_right = i32::from_le_bytes(data[16..20].try_into().unwrap());
    let padding_top = i32::from_le_bytes(data[20..24].try_into().unwrap());
    let padding_bottom = i32::from_le_bytes(data[24..28].try_into().unwrap());
    let colors_off = u32::from_le_bytes(data[28..32].try_into().unwrap()) as usize;

    let read_i32s = |off: usize, n: u8| -> Result<Vec<i32>, NinePatchError> {
        let n = n as usize;
        let end = off.checked_add(n * 4).ok_or(NinePatchError::InvalidChunk)?;
        if end > data.len() {
            return Err(NinePatchError::InvalidChunk);
        }
        let mut v = Vec::with_capacity(n);
        for i in 0..n {
            let s = off + i * 4;
            v.push(i32::from_le_bytes(data[s..s + 4].try_into().unwrap()));
        }
        Ok(v)
    };
    let read_u32s = |off: usize, n: u8| -> Result<Vec<u32>, NinePatchError> {
        let n = n as usize;
        let end = off.checked_add(n * 4).ok_or(NinePatchError::InvalidChunk)?;
        if end > data.len() {
            return Err(NinePatchError::InvalidChunk);
        }
        let mut v = Vec::with_capacity(n);
        for i in 0..n {
            let s = off + i * 4;
            v.push(u32::from_le_bytes(data[s..s + 4].try_into().unwrap()));
        }
        Ok(v)
    };

    Ok(NinePatchChunk {
        was_deserialized,
        num_x_divs,
        num_y_divs,
        num_colors,
        x_divs: read_i32s(x_off, num_x_divs)?,
        y_divs: read_i32s(y_off, num_y_divs)?,
        colors: read_u32s(colors_off, num_colors)?,
        padding_left,
        padding_right,
        padding_top,
        padding_bottom,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_nine_patch_name() {
        assert!(is_nine_patch_png("drawable/btn.9.png"));
        assert!(!is_nine_patch_png("drawable/btn.png"));
    }

    #[test]
    fn decode_without_nptc_passthrough() {
        // Minimal 1x1 RGBA PNG without npTc.
        let png = minimal_rgba_png(1, 1, &[255, 0, 0, 255]);
        let out = decode_nine_patch_png(&png).unwrap();
        assert_eq!(out, png);
    }

    fn minimal_rgba_png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
        let mut encoded = Vec::new();
        let mut encoder = Encoder::new(&mut encoded, w, h);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        let mut data = Vec::new();
        for _ in 0..(w * h) as usize {
            data.extend_from_slice(rgba);
        }
        writer.write_image_data(&data).unwrap();
        drop(writer);
        encoded
    }
}
