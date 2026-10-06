//! Scores as pictures, so a piece that exists only as a PDF or a scan can
//! still be read — by eye, by the model — when no MIDI or MusicXML of it can
//! be found.
//!
//! Drawn by macOS's own renderers (Core Graphics), in grey: a score needs no
//! colour, and grey keeps each page small.

/// The long edge of a picture. Enough for noteheads and accidentals to read
/// on a full page of music.
const LONG_EDGE: f64 = 2000.0;

pub struct Pages {
    /// Pages in the whole document.
    pub count: usize,
    /// (page number, PNG), for the pages asked for.
    pub images: Vec<(usize, Vec<u8>)>,
}

pub use platform::{pdf_pages, scan};

#[cfg(target_os = "macos")]
mod platform {
    use objc2_core_foundation::{CFData, CGPoint, CGRect, CGSize};
    use objc2_core_graphics::{
        CGBitmapContextCreate, CGColorRenderingIntent, CGColorSpace, CGContext, CGDataProvider, CGImage,
        CGImageAlphaInfo, CGInterpolationQuality, CGPDFBox, CGPDFDocument, CGPDFPage,
    };

    use super::{png_gray, Pages, LONG_EDGE};

    /// Pages `first` to `last` (from 1) of a PDF.
    pub fn pdf_pages(bytes: &[u8], first: usize, last: usize) -> Result<Pages, String> {
        let data = CFData::from_bytes(bytes);
        let provider = CGDataProvider::with_cf_data(Some(&data)).ok_or("Could not read the PDF.")?;
        let doc = CGPDFDocument::with_provider(Some(&provider)).ok_or("Could not read the PDF.")?;
        if CGPDFDocument::is_encrypted(Some(&doc)) && !CGPDFDocument::is_unlocked(Some(&doc)) {
            return Err("Password-protected PDFs can't be opened.".into());
        }
        let count = CGPDFDocument::number_of_pages(Some(&doc));
        if count == 0 {
            return Err("The PDF has no pages.".into());
        }
        if first > count {
            return Err(format!("This PDF has only {count} page{}.", if count == 1 { "" } else { "s" }));
        }

        let mut images = Vec::new();
        for number in first.max(1)..=last.min(count) {
            let page = CGPDFDocument::page(Some(&doc), number).ok_or("Could not read the page.")?;
            let crop = CGPDFPage::box_rect(Some(&page), CGPDFBox::CropBox);
            // A page may be stored sideways and marked to be turned for display.
            let turned = CGPDFPage::rotation_angle(Some(&page)).rem_euclid(180) != 0;
            let (w, h) = if turned {
                (crop.size.height, crop.size.width)
            } else {
                (crop.size.width, crop.size.height)
            };
            if w <= 0.0 || h <= 0.0 {
                continue;
            }
            let scale = LONG_EDGE / w.max(h);
            let png = paint(w * scale, h * scale, |ctx| {
                // The drawing transform turns the page upright but never
                // enlarges it, so it gets a box the page's own size and the
                // scale comes first.
                CGContext::scale_ctm(Some(ctx), scale, scale);
                let upright = CGPDFPage::drawing_transform(
                    Some(&page),
                    CGPDFBox::CropBox,
                    CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w, h)),
                    0,
                    true,
                );
                CGContext::concat_ctm(Some(ctx), upright);
                CGContext::draw_pdf_page(Some(ctx), Some(&page));
            })?;
            images.push((number, png));
        }
        Ok(Pages { count, images })
    }

    /// A photographed or scanned score (PNG or JPEG), in grey and no larger
    /// than a rendered page. Never enlarged.
    pub fn scan(bytes: &[u8]) -> Result<Vec<u8>, String> {
        let data = CFData::from_bytes(bytes);
        let provider = CGDataProvider::with_cf_data(Some(&data)).ok_or("Could not read the image.")?;
        let intent = CGColorRenderingIntent::RenderingIntentDefault;
        // SAFETY: a null decode array means "no remapping", as documented.
        let image = unsafe {
            if bytes.starts_with(b"\x89PNG") {
                CGImage::with_png_data_provider(Some(&provider), std::ptr::null(), true, intent)
            } else {
                CGImage::with_jpeg_data_provider(Some(&provider), std::ptr::null(), true, intent)
            }
        }
        .ok_or("Could not read the image (PNG or JPEG only).")?;
        let (w, h) = (CGImage::width(Some(&image)) as f64, CGImage::height(Some(&image)) as f64);
        if w < 1.0 || h < 1.0 {
            return Err("The image is empty.".into());
        }
        let scale = (LONG_EDGE / w.max(h)).min(1.0);
        let (pw, ph) = (w * scale, h * scale);
        paint(pw, ph, |ctx| {
            let all = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(pw.round(), ph.round()));
            CGContext::draw_image(Some(ctx), all, Some(&image));
        })
    }

    /// A white grey canvas of about `w` × `h` pixels, drawn on by `draw`, as PNG.
    fn paint(w: f64, h: f64, draw: impl FnOnce(&CGContext)) -> Result<Vec<u8>, String> {
        let (pw, ph) = ((w.round() as usize).max(1), (h.round() as usize).max(1));
        let mut pixels = vec![255u8; pw * ph];
        let space = CGColorSpace::new_device_gray();
        // SAFETY: `pixels` holds pw × ph one-byte pixels and outlives `ctx`,
        // which is dropped before the buffer is read.
        let ctx = unsafe {
            CGBitmapContextCreate(
                pixels.as_mut_ptr().cast(),
                pw,
                ph,
                8,
                pw,
                space.as_deref(),
                CGImageAlphaInfo::None.0,
            )
        }
        .ok_or("Could not set up rendering.")?;
        let full = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(pw as f64, ph as f64));
        CGContext::set_gray_fill_color(Some(&ctx), 1.0, 1.0);
        CGContext::fill_rect(Some(&ctx), full);
        CGContext::set_interpolation_quality(Some(&ctx), CGInterpolationQuality::High);
        draw(&ctx);
        drop(ctx);
        png_gray(&pixels, pw as u32, ph as u32)
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::Pages;

    pub fn pdf_pages(_bytes: &[u8], _first: usize, _last: usize) -> Result<Pages, String> {
        Err("Reading PDF sheet music is only supported on macOS.".into())
    }

    pub fn scan(_bytes: &[u8]) -> Result<Vec<u8>, String> {
        Err("Reading sheet music images is only supported on macOS.".into())
    }
}

fn png_gray(pixels: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(pixels).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    /// A one-page PDF (A4) with a black bar across the top.
    fn tiny_pdf() -> Vec<u8> {
        let content = "0 g 50 780 495 20 re f";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
        }
        let xref = pdf.len();
        pdf.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for o in offsets {
            pdf.extend(format!("{o:010} 00000 n \n").bytes());
        }
        pdf.extend(
            format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes(),
        );
        pdf
    }

    #[test]
    fn a_page_renders_upright_in_grey() {
        let pages = pdf_pages(&tiny_pdf(), 1, 5).unwrap();
        assert_eq!(pages.count, 1);
        assert_eq!(pages.images.len(), 1);
        let (number, png) = &pages.images[0];
        assert_eq!(*number, 1);

        let decoder = png::Decoder::new(std::io::Cursor::new(png.as_slice()));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (1413, 2000));
        // The bar is near the top of the page, so the top of the picture —
        // not the bottom — is where it must show.
        let row = |y: u32| &buf[(y * info.width) as usize..((y + 1) * info.width) as usize];
        let dark = |y: u32| row(y).iter().filter(|p| **p < 64).count();
        assert!(dark(2000 * 50 / 842) > 1000, "the bar should be drawn near the top");
        assert_eq!(dark(1900), 0, "the bottom should be blank");
    }

    #[test]
    fn a_large_photo_is_brought_down_to_page_size() {
        // A 3000 × 1000 white PNG with a dark left half.
        let (w, h) = (3000u32, 1000u32);
        let pixels: Vec<u8> = (0..w * h).map(|i| if i % w < w / 2 { 0 } else { 255 }).collect();
        let png = png_gray(&pixels, w, h).unwrap();
        let out = scan(&png).unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(out.as_slice())).read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (2000, 667));
        assert!(buf[10] < 64 && buf[1990] > 192, "the dark half stays on the left");
    }

    #[test]
    fn a_broken_pdf_is_an_error() {
        assert!(pdf_pages(b"%PDF-1.4 nothing here", 1, 1).is_err());
    }
}
