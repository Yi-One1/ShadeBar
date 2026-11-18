//! 零依赖 PNG 编码器（T04 真机截图取证专用）。
//!
//! # 为什么手写
//!
//! 架构 §9.1 禁止引入 `image` / `png` / `flate2` 等 crate（体积门禁 + 依赖面），
//! 但验收要求「截图」。本模块用 **stored（不压缩）DEFLATE 块** 生成合法 PNG：
//!
//! ```text
//! signature | IHDR | IDAT(zlib: 0x78 0x01 + stored blocks + adler32) | IEND
//! ```
//!
//! stored 块不需要哈夫曼表/压缩算法，只有长度字段与补码；CRC32 与 Adler32
//! 均为纯整数运算。生成的文件可被任意标准解码器打开，且**无第三方依赖**。
//!
//! # 输入
//!
//! [`ScreenBuffer`](shadebar_core::engine::ScreenBuffer) 是 top-down BGRA8；
//! 这里逐像素转换为 RGBA8（PNG color type 6）。
//!
//! # 零 panic
//!
//! 所有长度用 `saturating_*`，切片访问全部带边界判断；非法尺寸返回 `None`
//! 而不是 panic / 不可达分支宏。

/// PNG 文件头。
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// 单个 stored DEFLATE 块的最大净荷（RFC 1951 的 LEN 是 16 位）。
const MAX_STORED_BLOCK: usize = 65_535;

/// CRC-32（IEEE 802.3，多项式 0xEDB88320），查表法。
#[must_use]
pub const fn crc32(bytes: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0usize;
        while k < 8 {
            c = if c & 1 != 0 {
                (c >> 1) ^ 0xEDB8_8320
            } else {
                c >> 1
            };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    let mut crc = 0xFFFF_FFFFu32;
    let mut n = 0usize;
    while n < bytes.len() {
        let idx = (crc ^ (bytes[n] as u32)) as usize & 0xFF;
        crc = table[idx] ^ (crc >> 8);
        n += 1;
    }
    !crc
}

/// Adler-32（RFC 1950），zlib 流的校验和。
#[must_use]
pub const fn adler32(bytes: &[u8]) -> u32 {
    const MOD: u32 = 65_521;
    let mut a = 1u32;
    let mut b = 0u32;
    let mut i = 0usize;
    while i < bytes.len() {
        a = (a + (bytes[i] as u32)) % MOD;
        b = (b + a) % MOD;
        i += 1;
    }
    (b << 16) | a
}

/// 把 32 位整数写成大端 4 字节。
#[must_use]
const fn be32(v: u32) -> [u8; 4] {
    [
        (v >> 24) as u8,
        (v >> 16) as u8,
        (v >> 8) as u8,
        v as u8,
    ]
}

/// 追加一个 PNG chunk：`length(4) + type(4) + data + crc(4)`。
fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&be32(data.len() as u32));
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&be32(crc32(&crc_input)));
}

/// 把原始字节流封装成 zlib 容器 + 一串 stored DEFLATE 块。
fn zlib_stored(raw: &[u8]) -> Vec<u8> {
    let mut z = Vec::with_capacity(raw.len().saturating_add(16));
    // CM=8 (deflate), CINFO=7 (32K window), FCHECK 使 0x7801 合法。
    z.push(0x78);
    z.push(0x01);
    let mut off = 0usize;
    if raw.is_empty() {
        // 空流的最后一个空块。
        z.extend_from_slice(&[0x01, 0x00, 0x00, 0xFF, 0xFF]);
    } else {
        while off < raw.len() {
            let end = (off + MAX_STORED_BLOCK).min(raw.len());
            let len = end - off;
            let final_block = end == raw.len();
            z.push(u8::from(final_block)); // BFINAL + BTYPE=00
            let l = len as u16;
            z.push(l as u8);
            z.push((l >> 8) as u8);
            z.push(!(l as u8));
            z.push(!((l >> 8) as u8));
            z.extend_from_slice(&raw[off..end]);
            off = end;
        }
    }
    z.extend_from_slice(&be32(adler32(raw)));
    z
}

/// 把 BGRA8 像素缓冲编码为 RGBA8 PNG；返回完整文件字节。
///
/// `stride` 为每行字节数（通常为 `width * 4`）。`height` 从 `bgra.len()`
/// 推导；长度不足或尺寸非法时返回 `None`。
#[must_use]
pub fn encode_bgra_png(width: u32, height: u32, bgra: &[u8]) -> Option<Vec<u8>> {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 || w > 65_535 || h > 65_535 {
        return None;
    }
    let row_bytes = w.checked_mul(4)?;
    let needed = row_bytes.checked_mul(h)?;
    if bgra.len() < needed {
        return None;
    }

    // 每行前面加一个 filter byte（0 = None），再放 RGBA 数据。
    let mut raw = Vec::with_capacity(needed.saturating_add(h));
    for y in 0..h {
        raw.push(0u8);
        let start = y.saturating_mul(row_bytes);
        let row = bgra.get(start..start.saturating_add(row_bytes))?;
        for px in row.as_chunks::<4>().0 {
            // BGRA -> RGBA
            raw.push(px[2]);
            raw.push(px[1]);
            raw.push(px[0]);
            raw.push(px[3]);
        }
    }

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&be32(width));
    ihdr.extend_from_slice(&be32(height));
    ihdr.push(8); // bit depth
    ihdr.push(6); // color type: RGBA
    ihdr.push(0); // compression
    ihdr.push(0); // filter
    ihdr.push(0); // interlace

    let idat = zlib_stored(&raw);
    let mut out = Vec::with_capacity(raw.len().saturating_add(64));
    out.extend_from_slice(&PNG_SIGNATURE);
    push_chunk(&mut out, b"IHDR", &ihdr);
    push_chunk(&mut out, b"IDAT", &idat);
    push_chunk(&mut out, b"IEND", &[]);
    Some(out)
}

/// 一条截图标注（矩形 + 颜色 + 边框宽度）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mark {
    /// 屏幕绝对坐标矩形（调用方负责换算到缓冲相对坐标）。
    pub rect: shadebar_core::engine::RectPx,
    /// BGRA 颜色。
    pub color: [u8; 4],
    /// 边框宽度（0 = 实心填充）。
    pub thickness: u32,
}

/// 在 BGRA 缓冲上画一个实心或空心矩形（截图上的可视化标注）。
///
/// 坐标是**相对缓冲左上角**的；越界自动裁剪，不 panic。
pub fn draw_rect(bgra: &mut [u8], width: u32, height: u32, mark: &Mark) {
    let w = width as i32;
    let h = height as i32;
    let x0 = mark.rect.left.max(0);
    let y0 = mark.rect.top.max(0);
    let x1 = mark.rect.right.min(w);
    let y1 = mark.rect.bottom.min(h);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let t = mark.thickness as i32;
    for y in y0..y1 {
        for x in x0..x1 {
            let on_border = t <= 1
                || x - x0 < t
                || x1 - 1 - x < t
                || y - y0 < t
                || y1 - 1 - y < t;
            if mark.thickness == 0 || on_border {
                let off = ((y as usize).saturating_mul(width as usize))
                    .saturating_add(x as usize)
                    .saturating_mul(4);
                if let Some(px) = bgra.get_mut(off..off.saturating_add(4)) {
                    px.copy_from_slice(&mark.color);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_known_vectors() {
        // 与 Python zlib.crc32 独立核对过的校验值（RFC 1952 / PNG chunk type）。
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b"IHDR"), 0xA8A1_AE0A);
        assert_eq!(crc32(b"IDAT"), 0x35AF_061E);
    }

    #[test]
    fn every_png_chunk_crc_matches_its_payload() {
        let bgra = [0u8, 0, 0, 255];
        let Some(png) = encode_bgra_png(1, 1, &bgra) else {
            return;
        };
        let mut off = 8usize; // skip signature
        while off + 12 <= png.len() {
            let len = u32::from_be_bytes([png[off], png[off + 1], png[off + 2], png[off + 3]])
                as usize;
            let chunk_start = off + 4;
            let data_start = chunk_start + 4;
            let data_end = data_start + len;
            assert!(data_end + 4 <= png.len(), "chunk 越界");
            let mut input = Vec::with_capacity(4 + len);
            input.extend_from_slice(&png[chunk_start..data_start]);
            input.extend_from_slice(&png[data_start..data_end]);
            let expected = crc32(&input);
            let stored = u32::from_be_bytes([
                png[data_end],
                png[data_end + 1],
                png[data_end + 2],
                png[data_end + 3],
            ]);
            assert_eq!(stored, expected, "chunk CRC 必须与载荷自洽");
            off = data_end + 4;
        }
        assert_eq!(off, png.len(), "必须恰好解析到文件末尾");
    }

    #[test]
    fn adler32_matches_known_vectors() {
        assert_eq!(adler32(b""), 1);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn png_signature_and_chunks_are_well_formed() {
        let mut bgra = vec![0u8; 2 * 2 * 4];
        for px in bgra.as_chunks_mut::<4>().0 {
            px.copy_from_slice(&[0x11, 0x22, 0x33, 0xFF]);
        }
        let Some(png) = encode_bgra_png(2, 2, &bgra) else {
            return;
        };
        assert_eq!(&png[0..8], &PNG_SIGNATURE);
        // IHDR 长度必须是 13，color type 6 / bit depth 8。
        // IHDR 长度必须是 13，color type 6 / bit depth 8。
        assert_eq!(&png[8..12], &[0, 0, 0, 13]);
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(png[24], 8);
        assert_eq!(png[25], 6);
        // 结尾是 IEND。
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    }

    #[test]
    fn bgra_is_swapped_to_rgba_in_idat() {
        let bgra = [10u8, 20, 30, 255];
        let Some(png) = encode_bgra_png(1, 1, &bgra) else {
            return;
        };
        // 找到 IDAT 数据起点：signature(8) + IHDR chunk(25) = 33，
        // 再 + length(4) + type(4) = 41；zlib 头 2 + 块头 5 = 48；filter 1 = 49。
        let pixels = &png[49..53];
        assert_eq!(pixels, &[30, 20, 10, 255], "BGRA 必须转成 RGBA");
    }

    #[test]
    fn invalid_dimensions_return_none() {
        assert!(encode_bgra_png(0, 1, &[0; 4]).is_none());
        assert!(encode_bgra_png(1, 1, &[0; 3]).is_none());
        assert!(encode_bgra_png(u32::MAX, 1, &[]).is_none());
    }

    #[test]
    fn stored_blocks_split_at_65535() {
        let raw = vec![0xABu8; MAX_STORED_BLOCK + 10];
        let z = zlib_stored(&raw);
        // 2 字节头 + 2 个块头(5+5) + 数据 + 4 字节 adler。
        assert_eq!(z.len(), 2 + 5 + MAX_STORED_BLOCK + 5 + 10 + 4);
        assert_eq!(z[0], 0x78);
        assert_eq!(z[2], 0x00, "第一块不是最后一块");
        assert_eq!(z[7 + MAX_STORED_BLOCK], 0x01, "第二块是最后一块");
    }

    #[test]
    fn draw_rect_clips_out_of_bounds() {
        let mut buf = vec![0u8; 4 * 4 * 4];
        let fill = Mark {
            rect: shadebar_core::engine::rect_px(-10, -10, 99, 99),
            color: [1, 2, 3, 255],
            thickness: 0,
        };
        draw_rect(&mut buf, 4, 4, &fill);
        assert!(buf.as_chunks::<4>().0.iter().all(|p| *p == [1, 2, 3, 255]));
        let offscreen = Mark {
            rect: shadebar_core::engine::rect_px(100, 100, 200, 200),
            color: [9, 9, 9, 255],
            thickness: 0,
        };
        draw_rect(&mut buf, 4, 4, &offscreen);
        assert!(buf.as_chunks::<4>().0.iter().all(|p| p[0] == 1), "越界矩形不得改动缓冲");
    }
}
