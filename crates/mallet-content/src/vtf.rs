//! Valve Texture Format (VTF) decoding, versions 7.0 to 7.5.
//!
//! Only the first frame, face and depth slice of one mip level is decoded, which is all an
//! editor viewport needs.

/// A decoded texture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// Width of the full-size texture, used for texture coordinates.
    pub width: u32,
    /// Height of the full-size texture, used for texture coordinates.
    pub height: u32,
    /// Width of the decoded mip level.
    pub pixel_width: u32,
    /// Height of the decoded mip level.
    pub pixel_height: u32,
    /// RGBA pixels of the decoded mip level, row by row from the top.
    pub rgba: Vec<u8>,
}

const FLAG_ENVMAP: u32 = 0x4000;
const RESOURCE_HIGH_RES_IMAGE: [u8; 3] = [0x30, 0, 0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Rgba8888,
    Abgr8888,
    Rgb888,
    Bgr888,
    Rgb565,
    I8,
    Ia88,
    A8,
    Argb8888,
    Bgra8888,
    Dxt1,
    Dxt3,
    Dxt5,
    Bgrx8888,
    Bgr565,
    Bgra4444,
    Bgrx5551,
    Bgra5551,
    Uv88,
}

impl Format {
    fn from_id(id: i32) -> Option<Self> {
        Some(match id {
            0 => Self::Rgba8888,
            1 => Self::Abgr8888,
            2 | 9 => Self::Rgb888,
            3 | 10 => Self::Bgr888,
            4 => Self::Rgb565,
            5 => Self::I8,
            6 => Self::Ia88,
            8 => Self::A8,
            11 => Self::Argb8888,
            12 => Self::Bgra8888,
            13 | 20 => Self::Dxt1,
            14 => Self::Dxt3,
            15 => Self::Dxt5,
            16 => Self::Bgrx8888,
            17 => Self::Bgr565,
            18 => Self::Bgrx5551,
            19 => Self::Bgra4444,
            21 => Self::Bgra5551,
            22 => Self::Uv88,
            _ => return None,
        })
    }

    /// Bytes needed for one image of this size.
    fn size(self, width: u32, height: u32) -> usize {
        let (width, height) = (width as usize, height as usize);
        let blocks = width.div_ceil(4).max(1) * height.div_ceil(4).max(1);
        match self {
            Self::Dxt1 => blocks * 8,
            Self::Dxt3 | Self::Dxt5 => blocks * 16,
            Self::I8 | Self::A8 => width * height,
            Self::Ia88 | Self::Rgb565 | Self::Bgr565 | Self::Bgra4444 | Self::Bgrx5551 | Self::Bgra5551 | Self::Uv88 => {
                width * height * 2
            }
            Self::Rgb888 | Self::Bgr888 => width * height * 3,
            Self::Rgba8888 | Self::Abgr8888 | Self::Argb8888 | Self::Bgra8888 | Self::Bgrx8888 => width * height * 4,
        }
    }
}

/// Decodes the largest mip level whose sides are at most `max_side` pixels, or the
/// smallest available level if none fits.
pub fn decode(data: &[u8], max_side: u32) -> Result<Image, String> {
    let read_u16 = |at: usize| data.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let read_u32 = |at: usize| data.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if data.get(0..4) != Some(b"VTF\0") {
        return Err("not a VTF file".to_owned());
    }
    let truncated = || "VTF header is truncated".to_owned();
    let minor = read_u32(8).ok_or_else(truncated)?;
    let header_size = read_u32(12).ok_or_else(truncated)? as usize;
    let width = u32::from(read_u16(16).ok_or_else(truncated)?);
    let height = u32::from(read_u16(18).ok_or_else(truncated)?);
    let flags = read_u32(20).ok_or_else(truncated)?;
    let frames = usize::from(read_u16(24).ok_or_else(truncated)?.max(1));
    let format_id = read_u32(52).ok_or_else(truncated)? as i32;
    let mip_count = u32::from(*data.get(56).ok_or_else(truncated)?).max(1);
    let low_format_id = read_u32(57).ok_or_else(truncated)? as i32;
    let low_width = u32::from(*data.get(61).ok_or_else(truncated)?);
    let low_height = u32::from(*data.get(62).ok_or_else(truncated)?);
    let depth = if minor >= 2 { usize::from(read_u16(63).ok_or_else(truncated)?.max(1)) } else { 1 };
    let format = Format::from_id(format_id).ok_or_else(|| format!("unsupported VTF image format {format_id}"))?;
    let faces = if flags & FLAG_ENVMAP != 0 { 6 } else { 1 };

    let high_res_start = if minor >= 3 {
        let resource_count = read_u32(68).ok_or_else(truncated)? as usize;
        (0..resource_count)
            .find_map(|index| {
                let at = 80 + index * 8;
                (data.get(at..at + 3)? == RESOURCE_HIGH_RES_IMAGE).then(|| read_u32(at + 4)).flatten()
            })
            .ok_or("VTF has no image resource")? as usize
    } else {
        let low_res_size = Format::from_id(low_format_id).map_or(0, |low| {
            if low_width == 0 || low_height == 0 {
                0
            } else {
                low.size(low_width, low_height)
            }
        });
        header_size + low_res_size
    };

    let mip_size = |level: u32| ((width >> level).max(1), (height >> level).max(1));
    let level = (0..mip_count)
        .find(|&level| {
            let (w, h) = mip_size(level);
            w.max(h) <= max_side
        })
        .unwrap_or(mip_count - 1);
    let copies = frames * faces * depth;
    let skipped: usize = (level + 1..mip_count)
        .map(|smaller| {
            let (w, h) = mip_size(smaller);
            format.size(w, h) * copies
        })
        .sum();
    let (pixel_width, pixel_height) = mip_size(level);
    let start = high_res_start + skipped;
    let source = data
        .get(start..start + format.size(pixel_width, pixel_height))
        .ok_or("VTF image data is truncated")?;
    Ok(Image {
        width,
        height,
        pixel_width,
        pixel_height,
        rgba: to_rgba(format, source, pixel_width, pixel_height),
    })
}

fn to_rgba(format: Format, source: &[u8], width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let block = match format {
        Format::Dxt1 => Some(texpresso::Format::Bc1),
        Format::Dxt3 => Some(texpresso::Format::Bc2),
        Format::Dxt5 => Some(texpresso::Format::Bc3),
        _ => None,
    };
    if let Some(block) = block {
        let mut rgba = vec![0u8; w * h * 4];
        block.decompress(source, w, h, &mut rgba);
        return rgba;
    }
    let expand5 = |value: u16| ((value << 3) | (value >> 2)) as u8;
    let expand6 = |value: u16| ((value << 2) | (value >> 4)) as u8;
    let expand4 = |value: u16| ((value << 4) | value) as u8;
    let pixel_size = format.size(1, 1);
    source
        .chunks_exact(pixel_size)
        .take(w * h)
        .flat_map(|p| {
            let short = || u16::from_le_bytes([p[0], p[1]]);
            match format {
                Format::Rgba8888 => [p[0], p[1], p[2], p[3]],
                Format::Abgr8888 => [p[3], p[2], p[1], p[0]],
                Format::Argb8888 => [p[1], p[2], p[3], p[0]],
                Format::Bgra8888 => [p[2], p[1], p[0], p[3]],
                Format::Bgrx8888 => [p[2], p[1], p[0], 255],
                Format::Rgb888 => [p[0], p[1], p[2], 255],
                Format::Bgr888 => [p[2], p[1], p[0], 255],
                Format::I8 => [p[0], p[0], p[0], 255],
                Format::Ia88 => [p[0], p[0], p[0], p[1]],
                Format::A8 => [255, 255, 255, p[0]],
                Format::Uv88 => [p[0], p[1], 255, 255],
                Format::Rgb565 => {
                    let v = short();
                    [expand5(v & 31), expand6((v >> 5) & 63), expand5(v >> 11), 255]
                }
                Format::Bgr565 => {
                    let v = short();
                    [expand5(v >> 11), expand6((v >> 5) & 63), expand5(v & 31), 255]
                }
                Format::Bgra4444 => {
                    let v = short();
                    [expand4((v >> 8) & 15), expand4((v >> 4) & 15), expand4(v & 15), expand4(v >> 12)]
                }
                Format::Bgrx5551 | Format::Bgra5551 => {
                    let v = short();
                    let alpha = if format == Format::Bgrx5551 || v >> 15 == 1 { 255 } else { 0 };
                    [expand5((v >> 10) & 31), expand5((v >> 5) & 31), expand5(v & 31), alpha]
                }
                Format::Dxt1 | Format::Dxt3 | Format::Dxt5 => unreachable!("block formats return above"),
            }
        })
        .collect()
}
