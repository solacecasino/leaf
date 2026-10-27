//! Pictures the `image` crate doesn't decode, on systems without macOS's
//! ImageIO: JPEG XL, JPEG 2000, AVIF, PSD, ICNS and camera RAW in pure Rust,
//! and HEIC through the system's own decoder (libheif on Linux, Windows'
//! HEIF codec), since there's no HEVC decoder in Rust.

use image::RgbImage;

/// The most pixels Leaf decodes, as on macOS: a file's claimed size is
/// checked first, so a hostile one can't ask for gigabytes.
const MAX_PIXELS: u64 = 400_000_000;

fn checked(w: u32, h: u32) -> Result<(), String> {
    if w == 0 || h == 0 {
        return Err("an empty image".into());
    }
    if w as u64 * h as u64 > MAX_PIXELS {
        return Err(format!("a {w}×{h} image is bigger than Leaf will open"));
    }
    Ok(())
}

/// Decodes `bytes` by what they start with.
pub fn decode(bytes: &[u8]) -> Result<RgbImage, String> {
    let head = &bytes[..bytes.len().min(32)];
    if head.starts_with(&[0xff, 0x0a]) || head.get(4..12) == Some(b"JXL \r\n\x87\n") {
        jxl(bytes)
    } else if head.starts_with(b"\0\0\0\x0cjP  \r\n\x87\n") || head.starts_with(&[0xff, 0x4f, 0xff, 0x51]) {
        jpeg2000(bytes)
    } else if head.get(4..8) == Some(b"ftyp") {
        let brand = head.get(8..12).unwrap_or_default();
        if brand == b"avif" || brand == b"avis" { avif(bytes) } else { heic(bytes) }
    } else if head.starts_with(b"8BPS") {
        psd(bytes)
    } else if head.starts_with(b"icns") {
        icns(bytes)
    } else {
        raw(bytes)
    }
}

fn jxl(bytes: &[u8]) -> Result<RgbImage, String> {
    let image = jxl_oxide::JxlImage::builder().read(bytes).map_err(|e| e.to_string())?;
    let (w, h) = (image.width(), image.height());
    checked(w, h)?;
    let render = image.render_frame(0).map_err(|e| e.to_string())?;
    let mut stream = render.stream_no_alpha();
    let channels = stream.channels() as usize;
    let mut buf = vec![0u8; stream.width() as usize * stream.height() as usize * channels];
    stream.write_to_buffer(&mut buf);
    let (w, h) = (stream.width(), stream.height());
    let rgb: Vec<u8> = match channels {
        1 => buf.iter().flat_map(|&v| [v, v, v]).collect(),
        3 => buf,
        // CMYK's black channel: darken the colour by it.
        _ => buf.chunks_exact(channels).flat_map(|p| {
            let k = 255 - p[3] as u16;
            [p[0], p[1], p[2]].map(|c| (c as u16 * k / 255) as u8)
        }).collect(),
    };
    RgbImage::from_raw(w, h, rgb).ok_or_else(|| "a JPEG XL of the wrong size".into())
}

fn jpeg2000(bytes: &[u8]) -> Result<RgbImage, String> {
    let image = hayro_jpeg2000::Image::new(bytes, &hayro_jpeg2000::DecodeSettings::default()).map_err(|e| format!("{e:?}"))?;
    checked(image.width(), image.height())?;
    image::DynamicImage::from_decoder(image).map(|i| i.into_rgb8()).map_err(|e| e.to_string())
}

fn avif(bytes: &[u8]) -> Result<RgbImage, String> {
    use avif_decode::Image;
    let decoded = avif_decode::Decoder::from_avif(bytes).and_then(|d| d.to_image()).map_err(|e| e.to_string())?;
    let (w, h, rgb): (usize, usize, Vec<u8>) = match decoded {
        Image::Rgb8(i) => (i.width(), i.height(), i.pixels().flat_map(|p| [p.r, p.g, p.b]).collect()),
        Image::Rgb16(i) => (i.width(), i.height(), i.pixels().flat_map(|p| [p.r, p.g, p.b].map(|c| (c >> 8) as u8)).collect()),
        Image::Rgba8(i) => (i.width(), i.height(), i.pixels().flat_map(|p| [p.r, p.g, p.b].map(|c| over_white(c, p.a))).collect()),
        Image::Rgba16(i) => (
            i.width(),
            i.height(),
            i.pixels().flat_map(|p| [p.r, p.g, p.b].map(|c| over_white((c >> 8) as u8, (p.a >> 8) as u8))).collect(),
        ),
        Image::Gray8(i) => (i.width(), i.height(), i.pixels().flat_map(|p| [p.value(); 3]).collect()),
        Image::Gray16(i) => (i.width(), i.height(), i.pixels().flat_map(|p| [(p.value() >> 8) as u8; 3]).collect()),
    };
    checked(w as u32, h as u32)?;
    RgbImage::from_raw(w as u32, h as u32, rgb).ok_or_else(|| "an AVIF of the wrong size".into())
}

/// A colour with transparency, shown on white paper.
fn over_white(c: u8, a: u8) -> u8 {
    ((c as u16 * a as u16 + 255 * (255 - a as u16)) / 255) as u8
}

fn rgba_on_white(w: u32, h: u32, rgba: &[u8]) -> Result<RgbImage, String> {
    let rgb = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]].map(|c| over_white(c, p[3]))).collect();
    RgbImage::from_raw(w, h, rgb).ok_or_else(|| "an image of the wrong size".into())
}

fn psd(bytes: &[u8]) -> Result<RgbImage, String> {
    let psd = psd::Psd::from_bytes(bytes).map_err(|e| e.to_string())?;
    checked(psd.width(), psd.height())?;
    rgba_on_white(psd.width(), psd.height(), &psd.rgba())
}

fn icns(bytes: &[u8]) -> Result<RgbImage, String> {
    let family = icns::IconFamily::read(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let biggest = family.available_icons().into_iter().max_by_key(|t| t.pixel_width()).ok_or("an empty icon")?;
    let icon = family.get_icon_with_type(biggest).map_err(|e| e.to_string())?.convert_to(icns::PixelFormat::RGBA);
    rgba_on_white(icon.width(), icon.height(), icon.data())
}

/// Camera RAW (DNG, CR2, CR3, NEF, ARW, RAF, ORF, RW2...), developed the
/// usual way: demosaiced, white-balanced, in sRGB.
fn raw(bytes: &[u8]) -> Result<RgbImage, String> {
    let source = rawler::rawsource::RawSource::new_from_slice(bytes);
    let decoder = rawler::get_decoder(&source).map_err(|_| "not an image Leaf can read".to_string())?;
    let image = decoder.raw_image(&source, &rawler::decoders::RawDecodeParams::default(), false).map_err(|e| e.to_string())?;
    checked(image.width as u32, image.height as u32)?;
    let developed = rawler::imgop::develop::RawDevelop::default().develop_intermediate(&image).map_err(|e| e.to_string())?;
    developed.to_dynamic_image().map(|i| i.into_rgb8()).ok_or_else(|| "a RAW Leaf couldn't develop".into())
}

#[cfg(target_os = "linux")]
fn heic(bytes: &[u8]) -> Result<RgbImage, String> {
    heif::decode(bytes)
}

#[cfg(windows)]
fn heic(bytes: &[u8]) -> Result<RgbImage, String> {
    wic::decode(bytes)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn heic(_: &[u8]) -> Result<RgbImage, String> {
    Err("HEIC needs the system's decoder".into())
}

/// libheif, which most Linux desktops have (it's how they show iPhone
/// photos), looked for when the first HEIC opens.
#[cfg(target_os = "linux")]
mod heif {
    use super::{RgbImage, checked};
    use std::ffi::{c_char, c_int, c_void};
    use std::sync::OnceLock;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct HeifError {
        code: c_int,
        subcode: c_int,
        message: *const c_char,
    }

    const COLORSPACE_RGB: c_int = 1;
    const CHROMA_INTERLEAVED_RGB: c_int = 10;
    const CHANNEL_INTERLEAVED: c_int = 10;

    struct Lib {
        context_alloc: unsafe extern "C" fn() -> *mut c_void,
        context_free: unsafe extern "C" fn(*mut c_void),
        read_from_memory: unsafe extern "C" fn(*mut c_void, *const c_void, usize, *const c_void) -> HeifError,
        primary_handle: unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> HeifError,
        handle_release: unsafe extern "C" fn(*mut c_void),
        handle_width: unsafe extern "C" fn(*const c_void) -> c_int,
        handle_height: unsafe extern "C" fn(*const c_void) -> c_int,
        decode_image: unsafe extern "C" fn(*const c_void, *mut *mut c_void, c_int, c_int, *const c_void) -> HeifError,
        image_release: unsafe extern "C" fn(*mut c_void),
        plane: unsafe extern "C" fn(*const c_void, c_int, *mut c_int) -> *const u8,
        image_width: unsafe extern "C" fn(*const c_void, c_int) -> c_int,
        image_height: unsafe extern "C" fn(*const c_void, c_int) -> c_int,
    }

    static LIB: OnceLock<Option<Lib>> = OnceLock::new();

    fn lib() -> Option<&'static Lib> {
        LIB.get_or_init(|| {
            // SAFETY: dlopen and dlsym with C strings; each symbol is cast to
            // the signature libheif's header declares.
            unsafe {
                let handle = [c"libheif.so.1", c"libheif.so"]
                    .iter()
                    .map(|n| libc::dlopen(n.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL))
                    .find(|h| !h.is_null())?;
                macro_rules! sym {
                    ($name:literal) => {{
                        let s = libc::dlsym(handle, concat!($name, "\0").as_ptr().cast());
                        if s.is_null() {
                            return None;
                        }
                        std::mem::transmute(s)
                    }};
                }
                Some(Lib {
                    context_alloc: sym!("heif_context_alloc"),
                    context_free: sym!("heif_context_free"),
                    read_from_memory: sym!("heif_context_read_from_memory_without_copy"),
                    primary_handle: sym!("heif_context_get_primary_image_handle"),
                    handle_release: sym!("heif_image_handle_release"),
                    handle_width: sym!("heif_image_handle_get_width"),
                    handle_height: sym!("heif_image_handle_get_height"),
                    decode_image: sym!("heif_decode_image"),
                    image_release: sym!("heif_image_release"),
                    plane: sym!("heif_image_get_plane_readonly"),
                    image_width: sym!("heif_image_get_width"),
                    image_height: sym!("heif_image_get_height"),
                })
            }
        })
        .as_ref()
    }

    pub fn decode(bytes: &[u8]) -> Result<RgbImage, String> {
        let lib = lib().ok_or("HEIC pictures need libheif: install your system's libheif1 (or libheif) package")?;
        let ok = |e: HeifError| if e.code == 0 { Ok(()) } else { Err(format!("libheif couldn't read this picture (error {})", e.code)) };
        // SAFETY: libheif objects made here are released here, in reverse order.
        unsafe {
            let ctx = (lib.context_alloc)();
            if ctx.is_null() {
                return Err("out of memory".into());
            }
            let mut handle = std::ptr::null_mut();
            let mut img = std::ptr::null_mut();
            let result = (|| {
                ok((lib.read_from_memory)(ctx, bytes.as_ptr().cast(), bytes.len(), std::ptr::null()))?;
                ok((lib.primary_handle)(ctx, &mut handle))?;
                checked((lib.handle_width)(handle).max(0) as u32, (lib.handle_height)(handle).max(0) as u32)?;
                ok((lib.decode_image)(handle, &mut img, COLORSPACE_RGB, CHROMA_INTERLEAVED_RGB, std::ptr::null()))?;
                let (w, h) = ((lib.image_width)(img, CHANNEL_INTERLEAVED), (lib.image_height)(img, CHANNEL_INTERLEAVED));
                let mut stride = 0;
                let data = (lib.plane)(img, CHANNEL_INTERLEAVED, &mut stride);
                if data.is_null() || w <= 0 || h <= 0 {
                    return Err("libheif gave back no picture".to_string());
                }
                let (w, h, stride) = (w as usize, h as usize, stride as usize);
                let mut rgb = Vec::with_capacity(w * h * 3);
                for y in 0..h {
                    rgb.extend_from_slice(std::slice::from_raw_parts(data.add(y * stride), w * 3));
                }
                RgbImage::from_raw(w as u32, h as u32, rgb).ok_or_else(|| "a HEIC of the wrong size".to_string())
            })();
            if !img.is_null() {
                (lib.image_release)(img);
            }
            if !handle.is_null() {
                (lib.handle_release)(handle);
            }
            (lib.context_free)(ctx);
            result
        }
    }
}

/// Windows Imaging Component: decodes HEIC when Windows' HEIF and HEVC
/// extensions are installed (they come with most PCs), and anything else
/// Windows has a codec for.
#[cfg(windows)]
mod wic {
    use super::{RgbImage, checked};
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::System::Com::*;

    pub fn decode(bytes: &[u8]) -> Result<RgbImage, String> {
        let none = || "Windows can't read this picture: HEIC needs the HEIF and HEVC Video Extensions from the Microsoft Store".to_string();
        // SAFETY: COM calls on objects that live for this function.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(|_| none())?;
            let stream = factory.CreateStream().map_err(|_| none())?;
            stream.InitializeFromMemory(bytes).map_err(|_| none())?;
            let decoder = factory.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).map_err(|_| none())?;
            let frame = decoder.GetFrame(0).map_err(|_| none())?;
            let (mut w, mut h) = (0, 0);
            frame.GetSize(&mut w, &mut h).map_err(|_| none())?;
            checked(w, h)?;
            let converter = factory.CreateFormatConverter().map_err(|_| none())?;
            converter
                .Initialize(&frame, &GUID_WICPixelFormat24bppRGB, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)
                .map_err(|_| none())?;
            let mut rgb = vec![0u8; w as usize * h as usize * 3];
            converter.CopyPixels(std::ptr::null(), w * 3, &mut rgb).map_err(|_| none())?;
            RgbImage::from_raw(w, h, rgb).ok_or_else(none)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_bytes_are_refused_not_crashed_on() {
        assert!(decode(b"not a picture at all").is_err());
        assert!(decode(b"").is_err());
        assert!(decode(b"8BPS garbage").is_err());
        assert!(decode(b"\xff\x0a garbage").is_err());
    }

    #[test]
    fn white_paper_under_transparency() {
        assert_eq!(over_white(0, 0), 255);
        assert_eq!(over_white(0, 255), 0);
        assert_eq!(over_white(100, 255), 100);
    }
}
