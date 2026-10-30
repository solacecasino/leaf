//! PDFs on systems without Core Graphics' PDF renderer: Google's pdfium
//! (the renderer in Chrome), which the Windows and Linux downloads carry
//! next to Leaf. It's loaded when the first PDF opens, so Leaf starts
//! without it, and says so if it's missing.

use pdfium_render::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static PDFIUM: OnceLock<Result<Pdfium, String>> = OnceLock::new();

/// Where pdfium is looked for: `LEAF_PDFIUM` (a file, for testing), next
/// to Leaf, in a `lib` folder beside it, then wherever the system keeps
/// libraries.
fn candidates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = crate::series::env("PDFIUM").map(PathBuf::from).into_iter().collect();
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        out.push(Pdfium::pdfium_platform_library_name_at_path(&dir));
        out.push(Pdfium::pdfium_platform_library_name_at_path(&dir.join("lib")));
        out.push(Pdfium::pdfium_platform_library_name_at_path(&dir.join("../lib")));
    }
    out.push(Pdfium::pdfium_platform_library_name().into());
    out
}

fn pdfium() -> Result<&'static Pdfium, String> {
    PDFIUM
        .get_or_init(|| {
            for path in candidates() {
                if let Ok(bindings) = Pdfium::bind_to_library(&path) {
                    return Ok(Pdfium::new(bindings));
                }
            }
            Err("PDFs need pdfium, which comes with leaf9000: keep the pdfium library next to the leaf9000 program".into())
        })
        .as_ref()
        .map_err(Clone::clone)
}

pub struct Pdf {
    doc: Mutex<PdfDocument<'static>>,
    pages: usize,
}

// SAFETY: pdfium is bound with its thread-safe bindings, which serialise
// every call, and the document is only used behind the Mutex.
unsafe impl Send for Pdf {}
unsafe impl Sync for Pdf {}

impl Pdf {
    pub fn open(path: &Path) -> Result<Pdf, String> {
        let doc = pdfium()?.load_pdf_from_file(path, None).map_err(|e| match e {
            PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError) => "this PDF is locked with a password".to_string(),
            e => format!("not a PDF leaf9000 can read ({e})"),
        })?;
        let pages = doc.pages().len() as usize;
        Ok(Pdf { doc: Mutex::new(doc), pages })
    }

    pub fn pages(&self) -> usize {
        self.pages
    }

    /// Draws a page so it fits `target` the way a scanned page would.
    pub fn render(&self, page: usize, target: crate::loader::Target) -> Result<image::RgbImage, String> {
        let doc = self.doc.lock().unwrap_or_else(|e| e.into_inner());
        let p = doc.pages().get(page as PdfPageIndex).map_err(|_| "a missing page")?;
        let (pw, ph) = (p.width().value.max(1.0), p.height().value.max(1.0));
        let (w, h) = crate::loader::scaled_size(pw.round() as u32, ph.round() as u32, target);
        let config = PdfRenderConfig::new().set_target_size(w as Pixels, h as Pixels).set_clear_color(PdfColor::WHITE);
        let bitmap = p.render_with_config(&config).map_err(|e| e.to_string())?;
        let (bw, bh) = (bitmap.width() as u32, bitmap.height() as u32);
        let rgb = bitmap.as_rgba_bytes().chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        image::RgbImage::from_raw(bw, bh, rgb).ok_or_else(|| "pdfium drew a page of the wrong size".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::Target;
    use crate::state::Fit;

    /// Runs when pdfium is there: always in the release workflow, and on a
    /// Mac with `LEAF_PDFIUM` pointing at a pdfium library.
    #[test]
    fn draws_pdf_pages_with_pdfium() {
        let Ok(_) = pdfium() else {
            assert!(crate::series::env("REQUIRE_PDFIUM").is_none(), "pdfium isn't there");
            eprintln!("skipped: no pdfium");
            return;
        };
        let dir = std::env::temp_dir().join(format!("leaf-pdfium-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A two-page PDF, each page a black square on white.
        let pdf = dir.join("two.pdf");
        std::fs::write(&pdf, two_page_pdf()).unwrap();
        let book = Pdf::open(&pdf).unwrap();
        assert_eq!(book.pages(), 2);
        let page = book.render(1, Target { w: 300, h: 400, fit: Fit::Page }).unwrap();
        assert_eq!(page.dimensions(), (300, 400));
        assert_eq!(page.get_pixel(5, 5).0, [255, 255, 255]);
        assert!(page.get_pixel(150, 200).0[0] < 50, "the square is drawn");
        assert!(Pdf::open(&dir.join("missing.pdf")).is_err());
    }

    /// A minimal PDF written by hand, with a correct cross-reference table.
    pub fn two_page_pdf() -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Contents 5 0 R >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Contents 5 0 R >>".to_string(),
            "<< /Length 24 >>\nstream\n0 g 100 150 100 100 re f\nendstream".to_string(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for o in offsets {
            out.extend(format!("{o:010} 00000 n \n").bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes());
        out
    }
}
