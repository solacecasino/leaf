//! macOS's own decoders and renderers, so Leaf reads far more than it
//! compiles in, at no cost to its size:
//!
//! - **ImageIO** decodes any image the system can (HEIC, AVIF, JPEG XL,
//!   TIFF, PSD, camera RAW, ...) when Leaf's built-in decoders can't.
//! - **Core Graphics** draws PDF pages at exactly the size they're shown,
//!   so they're sharp at any window size.
//! - **Core Text** lays out text books (plain text, text EPUBs, and the
//!   RTF, Word, OpenDocument and HTML files AppKit can import) into pages.
//!
//! Plain C calls into the system frameworks; every object made here is
//! released here.

use image::RgbImage;
use std::ffi::{CString, c_void};
use std::path::Path;

type Ref = *const c_void;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

impl CGRect {
    fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        CGRect { origin: CGPoint { x, y }, size: CGSize { width: w, height: h } }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGAffineTransform {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    tx: f64,
    ty: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CFRange {
    location: isize,
    length: isize,
}

#[repr(C)]
struct ParagraphSetting {
    spec: u32,
    size: usize,
    value: *const c_void,
}

const UTF8: u32 = 0x0800_0100;
/// RGBX, one byte each, in that order in memory.
const RGBX: u32 = 5 | (4 << 12);
const CROP_BOX: i32 = 1;
const LINE_HEIGHT_MULTIPLE: u32 = 7;
const PARAGRAPH_SPACING: u32 = 11;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: Ref);
    fn CFDataCreate(alloc: Ref, bytes: *const u8, len: isize) -> Ref;
    fn CFStringCreateWithBytes(alloc: Ref, bytes: *const u8, len: isize, encoding: u32, external: u8) -> Ref;
    fn CFAttributedStringCreate(alloc: Ref, string: Ref, attributes: Ref) -> Ref;
    fn CFAttributedStringGetLength(string: Ref) -> isize;
    fn CFDictionaryCreate(alloc: Ref, keys: *const Ref, values: *const Ref, n: isize, kcb: *const c_void, vcb: *const c_void) -> Ref;
    // Real sizes (six and five pointers): a zero-sized declaration lets the
    // compiler hand out any address, and CFDictionary then reads garbage.
    static kCFTypeDictionaryKeyCallBacks: [usize; 6];
    static kCFTypeDictionaryValueCallBacks: [usize; 5];
    static kCFBooleanTrue: Ref;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGColorSpaceCreateDeviceRGB() -> Ref;
    fn CGBitmapContextCreate(data: *mut c_void, w: usize, h: usize, bits: usize, row: usize, space: Ref, info: u32) -> Ref;
    fn CGBitmapContextGetData(ctx: Ref) -> *mut u8;
    fn CGBitmapContextGetBytesPerRow(ctx: Ref) -> usize;
    fn CGContextSetRGBFillColor(ctx: Ref, r: f64, g: f64, b: f64, a: f64);
    fn CGContextFillRect(ctx: Ref, rect: CGRect);
    fn CGContextScaleCTM(ctx: Ref, sx: f64, sy: f64);
    fn CGContextConcatCTM(ctx: Ref, t: CGAffineTransform);
    fn CGContextDrawImage(ctx: Ref, rect: CGRect, image: Ref);
    fn CGImageGetWidth(image: Ref) -> usize;
    fn CGImageGetHeight(image: Ref) -> usize;
    fn CGDataProviderCreateWithFilename(path: *const i8) -> Ref;
    fn CGPDFDocumentCreateWithProvider(provider: Ref) -> Ref;
    fn CGPDFDocumentIsUnlocked(doc: Ref) -> bool;
    fn CGPDFDocumentGetNumberOfPages(doc: Ref) -> usize;
    fn CGPDFDocumentGetPage(doc: Ref, number: usize) -> Ref;
    fn CGPDFPageGetBoxRect(page: Ref, kind: i32) -> CGRect;
    fn CGPDFPageGetRotationAngle(page: Ref) -> i32;
    fn CGPDFPageGetDrawingTransform(page: Ref, kind: i32, rect: CGRect, rotate: i32, keep_aspect: bool) -> CGAffineTransform;
    fn CGContextDrawPDFPage(ctx: Ref, page: Ref);
    fn CGPathCreateWithRect(rect: CGRect, transform: *const c_void) -> Ref;
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    fn CGImageSourceCreateWithData(data: Ref, options: Ref) -> Ref;
    fn CGImageSourceGetType(source: Ref) -> Ref;
    fn CGImageSourceGetCount(source: Ref) -> usize;
    fn CGImageSourceCreateImageAtIndex(source: Ref, index: usize, options: Ref) -> Ref;
    fn CGImageSourceCopyPropertiesAtIndex(source: Ref, index: usize, options: Ref) -> Ref;
    static kCGImagePropertyPixelWidth: Ref;
    static kCGImagePropertyPixelHeight: Ref;
    static kCGImagePropertyOrientation: Ref;
}

#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    fn CTFontCreateWithName(name: Ref, size: f64, matrix: *const c_void) -> Ref;
    fn CTParagraphStyleCreate(settings: *const ParagraphSetting, count: usize) -> Ref;
    fn CTFramesetterCreateWithAttributedString(string: Ref) -> Ref;
    fn CTFramesetterCreateFrame(setter: Ref, range: CFRange, path: Ref, attributes: Ref) -> Ref;
    fn CTFrameGetVisibleStringRange(frame: Ref) -> CFRange;
    fn CTFrameDraw(frame: Ref, ctx: Ref);
    static kCTFontAttributeName: Ref;
    static kCTParagraphStyleAttributeName: Ref;
    static kCTForegroundColorFromContextAttributeName: Ref;
}

/// A Core Foundation object, released when dropped.
struct Owned(Ref);

impl Owned {
    fn new(r: Ref) -> Option<Self> {
        (!r.is_null()).then_some(Owned(r))
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: every Owned came from a Create/Copy call, so we own one reference.
        unsafe { CFRelease(self.0) };
    }
}

// SAFETY: the documents, framesetters and strings held here are immutable
// once made, and Leaf uses each one from one thread at a time (behind a Mutex).
unsafe impl Send for Owned {}
unsafe impl Sync for Owned {}

fn cfstring(s: &str) -> Option<Owned> {
    // SAFETY: valid UTF-8 bytes and length.
    Owned::new(unsafe { CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as isize, UTF8, 0) })
}

fn dictionary(pairs: &[(Ref, Ref)]) -> Option<Owned> {
    let keys: Vec<Ref> = pairs.iter().map(|p| p.0).collect();
    let values: Vec<Ref> = pairs.iter().map(|p| p.1).collect();
    // SAFETY: equal-length arrays of CF objects; the standard callbacks retain them.
    Owned::new(unsafe {
        CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            pairs.len() as isize,
            (&raw const kCFTypeDictionaryKeyCallBacks).cast(),
            (&raw const kCFTypeDictionaryValueCallBacks).cast(),
        )
    })
}

/// A blank RGBX bitmap of `w`×`h` filled with `paper`.
fn bitmap(w: usize, h: usize, paper: [f64; 3]) -> Option<Owned> {
    // SAFETY: plain object creation; the context allocates its own pixels.
    unsafe {
        let space = Owned::new(CGColorSpaceCreateDeviceRGB())?;
        let ctx = Owned::new(CGBitmapContextCreate(std::ptr::null_mut(), w, h, 8, 0, space.0, RGBX))?;
        CGContextSetRGBFillColor(ctx.0, paper[0], paper[1], paper[2], 1.0);
        CGContextFillRect(ctx.0, CGRect::new(0.0, 0.0, w as f64, h as f64));
        Some(ctx)
    }
}

/// Copies a bitmap context's pixels out as RGB. Row 0 of a bitmap context's
/// memory is the top of the picture.
fn pixels(ctx: &Owned, w: usize, h: usize) -> RgbImage {
    // SAFETY: the context owns w*h pixels of 4 bytes, `row` bytes per row.
    let (data, row) = unsafe { (CGBitmapContextGetData(ctx.0), CGBitmapContextGetBytesPerRow(ctx.0)) };
    let bytes = unsafe { std::slice::from_raw_parts(data, row * h) };
    let mut out = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for px in bytes[y * row..y * row + w * 4].as_chunks::<4>().0 {
            out.extend_from_slice(&px[..3]);
        }
    }
    RgbImage::from_raw(w as u32, h as u32, out).expect("sized to fit")
}

/// The most pixels Leaf lets ImageIO decode: a file's size is checked
/// before decoding, so a hostile one can't ask for gigabytes.
const MAX_PIXELS: usize = 400_000_000;

/// A number from an ImageIO properties dictionary.
unsafe fn number(dict: Ref, key: Ref) -> Option<i64> {
    let mut out = 0i64;
    // SAFETY: dict is a CFDictionary; the value, if any, is a CFNumber.
    unsafe {
        let value = CFDictionaryGetValue(dict, key);
        (!value.is_null() && CFNumberGetValue(value, 4, (&raw mut out).cast())).then_some(out)
    }
}

/// Decodes any image format macOS knows, turned the way its EXIF says.
pub fn decode(bytes: &[u8]) -> Result<RgbImage, String> {
    // SAFETY: CF objects made and released here; the data is copied by CFDataCreate.
    unsafe {
        let data = Owned::new(CFDataCreate(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize)).ok_or("out of memory")?;
        let source = Owned::new(CGImageSourceCreateWithData(data.0, std::ptr::null())).ok_or("not an image")?;
        // Asking ImageIO for a picture of data it didn't recognise crashes
        // inside ImageIO (macOS 27), so check it recognised something first.
        if CGImageSourceGetType(source.0).is_null() || CGImageSourceGetCount(source.0) == 0 {
            return Err("not an image macOS can read".into());
        }
        // Its claimed size first, before any decoding.
        let props = Owned::new(CGImageSourceCopyPropertiesAtIndex(source.0, 0, std::ptr::null())).ok_or("not an image macOS can read")?;
        let claimed_w = number(props.0, kCGImagePropertyPixelWidth).unwrap_or(0).max(0) as usize;
        let claimed_h = number(props.0, kCGImagePropertyPixelHeight).unwrap_or(0).max(0) as usize;
        if claimed_w == 0 || claimed_h == 0 {
            return Err("not an image macOS can read".into());
        }
        if claimed_w.saturating_mul(claimed_h) > MAX_PIXELS {
            return Err(format!("a {claimed_w}×{claimed_h} image is bigger than Leaf will open"));
        }
        let orientation = number(props.0, kCGImagePropertyOrientation).unwrap_or(1);
        let image = Owned::new(CGImageSourceCreateImageAtIndex(source.0, 0, std::ptr::null())).ok_or("not an image macOS can read")?;
        let (w, h) = (CGImageGetWidth(image.0), CGImageGetHeight(image.0));
        if w == 0 || h == 0 || w.saturating_mul(h) > MAX_PIXELS {
            return Err("not an image macOS can read".into());
        }
        let ctx = bitmap(w, h, [1.0, 1.0, 1.0]).ok_or("out of memory")?;
        CGContextDrawImage(ctx.0, CGRect::new(0.0, 0.0, w as f64, h as f64), image.0);
        let img = pixels(&ctx, w, h);
        // Photos (HEIC from a phone, say) say which way up they go.
        Ok(match orientation {
            3 => image::imageops::rotate180(&img),
            6 => image::imageops::rotate90(&img),
            8 => image::imageops::rotate270(&img),
            _ => img,
        })
    }
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDictionaryGetValue(dict: Ref, key: Ref) -> Ref;
    /// Kind 4 is a 64-bit integer.
    fn CFNumberGetValue(number: Ref, kind: isize, out: *mut c_void) -> bool;
}

// ---- PDF ---------------------------------------------------------------------

pub struct Pdf {
    doc: Owned,
    pages: usize,
}

impl Pdf {
    pub fn open(path: &Path) -> Result<Pdf, String> {
        let c = CString::new(path.as_os_str().as_encoded_bytes()).map_err(|_| "a path with a zero byte")?;
        // SAFETY: a valid C path; objects released by Owned.
        unsafe {
            let provider = Owned::new(CGDataProviderCreateWithFilename(c.as_ptr())).ok_or("couldn't read the PDF")?;
            let doc = Owned::new(CGPDFDocumentCreateWithProvider(provider.0)).ok_or("not a PDF macOS can read")?;
            if !CGPDFDocumentIsUnlocked(doc.0) {
                return Err("this PDF is locked with a password".into());
            }
            let pages = CGPDFDocumentGetNumberOfPages(doc.0);
            Ok(Pdf { doc, pages })
        }
    }

    pub fn pages(&self) -> usize {
        self.pages
    }

    /// The page's size in points, turned as it's meant to be shown.
    fn size(&self, page: usize) -> Option<(f64, f64)> {
        // SAFETY: page numbers are 1-based and checked; pages belong to the document.
        unsafe {
            let p = CGPDFDocumentGetPage(self.doc.0, page + 1);
            if p.is_null() {
                return None;
            }
            let r = CGPDFPageGetBoxRect(p, CROP_BOX);
            let turned = CGPDFPageGetRotationAngle(p).rem_euclid(180) == 90;
            Some(if turned { (r.size.height, r.size.width) } else { (r.size.width, r.size.height) })
        }
    }

    /// Draws a page so it fits `target` the way a scanned page would.
    pub fn render(&self, page: usize, target: crate::loader::Target) -> Result<RgbImage, String> {
        let (pw, ph) = self.size(page).ok_or("a missing page")?;
        let (w, h) = crate::loader::scaled_size(pw.round().max(1.0) as u32, ph.round().max(1.0) as u32, target);
        let (w, h) = (w as usize, h as usize);
        let scale = w as f64 / pw;
        // SAFETY: the page belongs to the document; the context is ours.
        unsafe {
            let p = CGPDFDocumentGetPage(self.doc.0, page + 1);
            let ctx = bitmap(w, h, [1.0, 1.0, 1.0]).ok_or("out of memory")?;
            CGContextScaleCTM(ctx.0, scale, scale);
            // At the page's own size the transform only turns and moves it.
            let t = CGPDFPageGetDrawingTransform(p, CROP_BOX, CGRect::new(0.0, 0.0, pw, ph), 0, true);
            CGContextConcatCTM(ctx.0, t);
            CGContextDrawPDFPage(ctx.0, p);
            Ok(pixels(&ctx, w, h))
        }
    }
}

// ---- Text ----------------------------------------------------------------------

/// The page text is laid out on, in points. Pages are fixed so a book has
/// the same page numbers at any window size; they're drawn at the size shown.
const PAGE: (f64, f64) = (540.0, 780.0);
const MARGIN: f64 = 46.0;
const PAPER: [f64; 3] = [0.98, 0.97, 0.94];
const INK: [f64; 3] = [0.13, 0.12, 0.11];

pub struct Text {
    setter: Owned,
    /// Where each page starts and how much it holds, in UTF-16 units.
    pages: Vec<(isize, isize)>,
}

impl Text {
    /// Plain text, set in a book face at a reading size.
    pub fn from_plain(text: &str) -> Result<Text, String> {
        let string = cfstring(text).ok_or("couldn't read the text")?;
        // SAFETY: CF objects made and released here.
        unsafe {
            let face = cfstring("Iowan Old Style").ok_or("out of memory")?;
            let font = Owned::new(CTFontCreateWithName(face.0, 17.0, std::ptr::null())).ok_or("no font")?;
            let (line, para) = (1.35f64, 9.0f64);
            let settings = [
                ParagraphSetting { spec: LINE_HEIGHT_MULTIPLE, size: 8, value: (&raw const line).cast() },
                ParagraphSetting { spec: PARAGRAPH_SPACING, size: 8, value: (&raw const para).cast() },
            ];
            let style = Owned::new(CTParagraphStyleCreate(settings.as_ptr(), settings.len())).ok_or("out of memory")?;
            let attributes = dictionary(&[
                (kCTFontAttributeName, font.0),
                (kCTParagraphStyleAttributeName, style.0),
                (kCTForegroundColorFromContextAttributeName, kCFBooleanTrue),
            ])
            .ok_or("out of memory")?;
            let attributed = Owned::new(CFAttributedStringCreate(std::ptr::null(), string.0, attributes.0)).ok_or("out of memory")?;
            Self::paginate(attributed)
        }
    }

    /// A document AppKit can import with its own formatting: RTF, Word
    /// (.doc, .docx), OpenDocument (.odt), HTML and web archives.
    /// Call on the main thread.
    pub fn from_document(path: &Path) -> Result<Text, String> {
        use objc2::rc::{Allocated, Retained};
        use objc2::runtime::AnyObject;
        use objc2::{class, msg_send};
        use objc2_foundation::NSString;
        // SAFETY: standard AppKit calls; the result is an NSAttributedString,
        // which is the same object as a CFAttributedString.
        unsafe {
            let path = NSString::from_str(&path.to_string_lossy());
            let url: Retained<AnyObject> = msg_send![class!(NSURL), fileURLWithPath: &*path];
            let options: Retained<AnyObject> = msg_send![class!(NSDictionary), dictionary];
            let blank: Allocated<AnyObject> = msg_send![class!(NSAttributedString), alloc];
            let string: Option<Retained<AnyObject>> = msg_send![
                blank,
                initWithURL: &*url,
                options: &*options,
                documentAttributes: std::ptr::null_mut::<*mut AnyObject>(),
                error: std::ptr::null_mut::<*mut AnyObject>()
            ];
            let string = string.ok_or("macOS couldn't read this document")?;
            let ptr = Retained::into_raw(string) as Ref;
            Self::paginate(Owned(ptr))
        }
    }

    fn paginate(attributed: Owned) -> Result<Text, String> {
        // SAFETY: CF objects made and released here.
        unsafe {
            let total = CFAttributedStringGetLength(attributed.0);
            if total == 0 {
                return Err("the book is empty".into());
            }
            let setter = Owned::new(CTFramesetterCreateWithAttributedString(attributed.0)).ok_or("out of memory")?;
            let path = Owned::new(CGPathCreateWithRect(text_rect(), std::ptr::null())).ok_or("out of memory")?;
            let mut pages = Vec::new();
            let mut start = 0;
            while start < total {
                let frame = Owned::new(CTFramesetterCreateFrame(setter.0, CFRange { location: start, length: 0 }, path.0, std::ptr::null()))
                    .ok_or("couldn't lay out the text")?;
                let len = CTFrameGetVisibleStringRange(frame.0).length;
                if len <= 0 {
                    break; // something too big for a page; stop rather than loop
                }
                pages.push((start, len));
                start += len;
            }
            Ok(Text { setter, pages })
        }
    }

    pub fn pages(&self) -> usize {
        self.pages.len()
    }

    pub fn render(&self, page: usize, target: crate::loader::Target) -> Result<RgbImage, String> {
        let &(start, len) = self.pages.get(page).ok_or("a missing page")?;
        let (w, h) = crate::loader::scaled_size(PAGE.0 as u32, PAGE.1 as u32, target);
        let (w, h) = (w as usize, h as usize);
        // SAFETY: CF objects made and released here.
        unsafe {
            let ctx = bitmap(w, h, PAPER).ok_or("out of memory")?;
            CGContextScaleCTM(ctx.0, w as f64 / PAGE.0, h as f64 / PAGE.1);
            CGContextSetRGBFillColor(ctx.0, INK[0], INK[1], INK[2], 1.0);
            let path = Owned::new(CGPathCreateWithRect(text_rect(), std::ptr::null())).ok_or("out of memory")?;
            let frame = Owned::new(CTFramesetterCreateFrame(self.setter.0, CFRange { location: start, length: len }, path.0, std::ptr::null()))
                .ok_or("couldn't lay out the text")?;
            CTFrameDraw(frame.0, ctx.0);
            Ok(pixels(&ctx, w, h))
        }
    }
}

fn text_rect() -> CGRect {
    CGRect::new(MARGIN, MARGIN, PAGE.0 - 2.0 * MARGIN, PAGE.1 - 2.0 * MARGIN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::Target;
    use crate::state::Fit;

    #[test]
    fn decodes_through_imageio() {
        // A 2x1 PNG made with the image crate, decoded by macOS instead.
        let img = RgbImage::from_fn(2, 1, |x, _| if x == 0 { image::Rgb([255, 0, 0]) } else { image::Rgb([0, 0, 255]) });
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let back = decode(&png).unwrap();
        assert_eq!(back.dimensions(), (2, 1));
        assert!(back.get_pixel(0, 0)[0] > 200 && back.get_pixel(1, 0)[2] > 200);
        assert!(decode(b"not an image").is_err());
    }

    #[test]
    fn lays_out_text_into_pages() {
        let text = "A paragraph of a book that goes on for a while. ".repeat(400);
        let book = Text::from_plain(&text).unwrap();
        assert!(book.pages() > 3, "{} pages", book.pages());
        let page = book.render(0, Target { w: 540, h: 780, fit: Fit::Page }).unwrap();
        assert_eq!(page.dimensions(), (540, 780));
        // Some ink on the paper.
        assert!(page.pixels().any(|p| p[0] < 100));
        // The same layout for every page size.
        let small = book.render(1, Target { w: 270, h: 390, fit: Fit::Page }).unwrap();
        assert_eq!(small.dimensions(), (270, 390));
    }
}
