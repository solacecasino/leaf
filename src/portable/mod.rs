//! What macOS reads for Leaf, read by Leaf itself on Windows and Linux:
//! PDFs (pdfium), text books (cosmic-text), documents, RAR/7-Zip/TAR, and
//! the pictures the `image` crate doesn't decode.

pub mod archives;
pub mod docs;
pub mod images;
pub mod pdf;
pub mod text;
