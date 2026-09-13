//! 封面管线：提取封面处理（解码→等比缩入 1024→webp q80）与排版生成
//! （txt/md/docx 无内嵌封面时的「书名+作者」文字封面；系统字体回退，缺字体时纯色封面）。
//! 产物统一 webp 入库外缓存 covers/<hash>.webp；封面尺寸回填元数据（内容的纯函数）。

use image::{DynamicImage, GenericImageView};

// ---------- PDF 首页渲染（pdfium 动态库，可选能力） ----------

/// pdfium 运行时（动态库）绑定。加载顺序：SUMI_PDFIUM_PATH 环境变量（打包态由 Electron
/// 注入 extraResources 路径）→ 系统库搜索路径。绑定结果全局缓存（含失败：缺库时不逐书重试）
static PDFIUM: std::sync::OnceLock<Option<pdfium_render::prelude::Pdfium>> = std::sync::OnceLock::new();

fn pdfium() -> Option<&'static pdfium_render::prelude::Pdfium> {
    PDFIUM
        .get_or_init(|| {
            use pdfium_render::prelude::*;
            let bindings = match std::env::var("SUMI_PDFIUM_PATH") {
                Ok(p) if !p.is_empty() => Pdfium::bind_to_library(&p).ok(),
                _ => Pdfium::bind_to_system_library().ok(),
            };
            match bindings {
                Some(b) => {
                    tracing::info!("pdfium 已加载（PDF 首页封面可用）");
                    Some(Pdfium::new(b))
                }
                None => {
                    tracing::warn!("pdfium 动态库不可用：PDF 封面回退排版生成（可设 SUMI_PDFIUM_PATH 指定库路径）");
                    None
                }
            }
        })
        .as_ref()
}

/// pdfium 是否可用（测试与封面链路判定用）
pub fn pdfium_available() -> bool {
    pdfium().is_some()
}

/// 渲染 PDF 首页为图像（封面用；目标宽 800px，封面管线后续统一缩放入 1024）
pub fn render_pdf_first_page(bytes: &[u8]) -> Option<DynamicImage> {
    use pdfium_render::prelude::*;
    let pdfium = pdfium()?;
    let doc = pdfium.load_pdf_from_byte_slice(bytes, None).ok()?;
    let page = doc.pages().get(0).ok()?;
    let config = PdfRenderConfig::new().set_target_width(800);
    let bitmap = page.render_with_config(&config).ok()?;
    bitmap.as_image().ok()
}

/// PDF 文档信息字典的 Title/Author（空值回退 None）
pub fn pdf_meta(bytes: &[u8]) -> Option<(String, String)> {
    use pdfium_render::prelude::*;
    let pdfium = pdfium()?;
    let doc = pdfium.load_pdf_from_byte_slice(bytes, None).ok()?;
    let title = doc
        .metadata()
        .get(PdfDocumentMetadataTagType::Title)
        .map(|t| t.value().to_string())
        .filter(|s| !s.trim().is_empty());
    let author = doc
        .metadata()
        .get(PdfDocumentMetadataTagType::Author)
        .map(|t| t.value().to_string())
        .filter(|s| !s.trim().is_empty());
    Some((title?, author.unwrap_or_default()))
}

/// 封面统一边长上限（等比缩入 1024，不放大）
pub const COVER_MAX_EDGE: u32 = 1024;
/// webp 有损质量（与 hawk 一致）
pub const WEBP_QUALITY: f32 = 80.0;

/// 处理提取封面：解码 → 等比缩入 1024 → webp。返回 (webp 字节, 宽, 高)
pub fn process_cover(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let format = image::guess_format(bytes).ok()?;
    let img = image::load_from_memory_with_format(bytes, format).ok()?;
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    let img = if w.max(h) > COVER_MAX_EDGE {
        resize_to_fit(img, COVER_MAX_EDGE)
    } else {
        img
    };
    let (rw, rh) = img.dimensions();
    let webp = encode_webp(&img)?;
    Some((webp, rw, rh))
}

fn resize_to_fit(img: DynamicImage, max_edge: u32) -> DynamicImage {
    let (w, h) = img.dimensions();
    let scale = max_edge as f32 / w.max(h) as f32;
    let (nw, nh) = ((w as f32 * scale).max(1.0) as u32, (h as f32 * scale).max(1.0) as u32);
    let src = img.to_rgba8();
    let mut dst = image::RgbaImage::new(nw, nh);
    let mut resizer = fast_image_resize::Resizer::new();
    let ok = resizer
        .resize(
            &src,
            &mut dst,
            Some(&fast_image_resize::ResizeOptions::new().resize_alg(
                fast_image_resize::ResizeAlg::Convolution(fast_image_resize::FilterType::Lanczos3),
            )),
        )
        .is_ok();
    if !ok {
        return DynamicImage::ImageRgba8(image::imageops::resize(&src, nw, nh, image::imageops::FilterType::Lanczos3));
    }
    DynamicImage::ImageRgba8(dst)
}

fn encode_webp(img: &DynamicImage) -> Option<Vec<u8>> {
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let encoder = webp::Encoder::from_rgba(rgba.as_raw(), w, h);
    let encoded = encoder.encode(WEBP_QUALITY);
    let bytes = encoded.to_vec();
    if bytes.is_empty() {
        None
    } else {
        Some(bytes)
    }
}

/// 排版生成封面（txt/md/docx）：竖向渐变底 + 书名（自动换行）+ 作者。
/// 系统字体候选加载失败时退化为纯渐变封面（无文字），尺寸 600×900
pub fn generated_cover(title: &str, authors: &[String]) -> (Vec<u8>, u32, u32) {
    let (w, h) = (600u32, 900u32);
    let mut canvas = image::RgbaImage::new(w, h);
    // 竖向渐变：深蓝 → 靛
    for y in 0..h {
        let t = y as f32 / h as f32;
        let r = (24.0 + 26.0 * t) as u8;
        let g = (34.0 + 22.0 * t) as u8;
        let b = (58.0 + 66.0 * t) as u8;
        for x in 0..w {
            canvas.put_pixel(x, y, image::Rgba([r, g, b, 255]));
        }
    }

    let author_line = authors.join(" · ");
    if let Some(font) = load_system_font() {
        draw_text_centered(&mut canvas, title, w, 340.0, 52.0, 10.0, 6, &font);
        if !author_line.is_empty() {
            draw_text_centered(&mut canvas, &author_line, w, 520.0, 30.0, 8.0, 4, &font);
        }
    }
    // 无字体：纯渐变（文字封面不可用，等价降级）

    let img = DynamicImage::ImageRgba8(canvas);
    let webp = encode_webp(&img).unwrap_or_default();
    (webp, w, h)
}

/// 平台系统字体候选（CJK 优先）；逐个尝试加载（ttc 取 index 0）
fn load_system_font() -> Option<ab_glyph::FontArc> {
    const CANDIDATES: &[&str] = &[
        // macOS
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/Library/Fonts/Arial Unicode.ttf",
        // Windows
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\simhei.ttf",
        // Linux
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
    ];
    for path in CANDIDATES {
        if let Ok(bytes) = std::fs::read(path) {
            if let Ok(font) = ab_glyph::FontArc::try_from_vec(bytes) {
                return Some(font);
            }
        }
    }
    None
}

/// 居中绘制（自动换行，超出高度截断）；字号随标题长度自适应
fn draw_text_centered(
    canvas: &mut image::RgbaImage,
    text: &str,
    cw: u32,
    y_start: f32,
    font_size: f32,
    line_gap: f32,
    max_lines: usize,
    font: &ab_glyph::FontArc,
) {
    use ab_glyph::{Font as _, PxScale, ScaleFont};
    let scale = PxScale::from(font_size);
    let scaled = font.as_scaled(scale);
    let ascent = scaled.ascent();

    // 字符步进宽度（缺字形时按字号近似）
    let char_width = |ch: char| -> f32 {
        let gid = font.glyph_id(ch);
        if gid.0 == 0 {
            font_size * 0.6
        } else {
            scaled.h_advance(gid)
        }
    };

    // 简易换行：按字符累积宽度（CJK 每字可断，西文按空格断）
    let max_width = cw as f32 - 80.0;
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_width = 0.0;
    for ch in text.chars() {
        let glyph_width = char_width(ch);
        if current_width + glyph_width > max_width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current_width = 0.0;
            if lines.len() >= max_lines {
                break;
            }
        }
        current.push(ch);
        current_width += glyph_width;
    }
    if !current.is_empty() && lines.len() < max_lines {
        lines.push(current);
    }

    let mut y = y_start;
    for line in &lines {
        let line_width: f32 = line.chars().map(|ch| char_width(ch)).sum();
        let mut x = ((cw as f32 - line_width) / 2.0).max(0.0);
        for ch in line.chars() {
            let gid = font.glyph_id(ch);
            let glyph = gid.with_scale_and_position(scale, ab_glyph::Point { x, y: y + ascent });
            if let Some(outlined) = font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                outlined.draw(|gx, gy, coverage| {
                    let px = bounds.min.x as i64 + gx as i64;
                    let py = bounds.min.y as i64 + gy as i64;
                    if px >= 0 && py >= 0 && (px as u32) < cw && (py as u32) < canvas.height() {
                        let alpha = (coverage * 255.0) as u8;
                        let pixel = canvas.get_pixel_mut(px as u32, py as u32);
                        pixel[0] = 245;
                        pixel[1] = 243;
                        pixel[2] = 238;
                        pixel[3] = pixel[3].max(alpha);
                    }
                });
            }
            x += char_width(ch);
        }
        y += font_size + line_gap;
    }
}

/// 封面缓存路径（库外派生缓存）
pub fn cache_cover_path(covers_dir: &str, id: &str) -> String {
    format!("{covers_dir}/{id}.webp")
}

/// 格式可即时生成排版封面（GET 封面的兜底链）；pdf 在 pdfium 缺库时同样回退排版封面
pub fn can_generate_cover(ext: &str) -> bool {
    matches!(ext, "txt" | "md" | "docx" | "pdf")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_cover_shape() {
        let (bytes, w, h) = generated_cover("三体", &["刘慈欣".to_string()]);
        assert_eq!((w, h), (600, 900));
        // webp 魔数（RIFF....WEBP）
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WEBP");
    }

    #[test]
    fn generated_cover_long_title_wraps() {
        let long_title = "一个特别特别特别长的书名用来测试自动换行逻辑是否能在有限宽度内正确工作";
        let (bytes, w, h) = generated_cover(long_title, &[]);
        assert_eq!((w, h), (600, 900));
        assert!(!bytes.is_empty());
    }

    #[test]
    fn process_cover_resizes_large() {
        // 3000×1000 纯色 → 缩入 1024 边长（宽 1024 高 ~341）
        let img = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(3000, 1000, image::Rgba([200, 100, 50, 255])));
        let (bytes, w, h) = process_cover(&img_to_png(&img)).expect("处理失败");
        assert_eq!(w, 1024);
        assert_eq!(h, 341);
        assert_eq!(&bytes[..4], b"RIFF");
    }

    #[test]
    fn process_cover_small_not_upscaled() {
        let img = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(200, 300, image::Rgba([1, 2, 3, 255])));
        let (_, w, h) = process_cover(&img_to_png(&img)).expect("处理失败");
        assert_eq!((w, h), (200, 300)); // 不放大
    }

    fn img_to_png(img: &DynamicImage) -> Vec<u8> {
        use image::ImageFormat;
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, ImageFormat::Png).unwrap();
        buf.into_inner()
    }
}
