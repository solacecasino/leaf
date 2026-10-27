#!/bin/bash
# Makes a book in every format this system's tools can write, in
# test-books/, for `leaf --check` to open: CBZ, an EPUB novel with a
# picture, and plain text always; TAR and 7-Zip where bsdtar is; RAR where
# RARLAB's rar is; on macOS, PDFs (a scan and a text one), RTF, Word (.docx
# and .doc), OpenDocument and HTML. The release workflow makes them on a
# Mac and opens them with every download. Usage: scripts/test-books.sh
set -e
cd "$(dirname "$0")/.."
cargo run --quiet --release --example sample >/dev/null
rm -rf test-books && mkdir -p test-books/pages
cp "samples/Demo Manga.cbz" test-books/manga.cbz
(cd test-books/pages && unzip -q "../manga.cbz")
tar -cf test-books/manga.cbt -C test-books/pages .
if command -v bsdtar >/dev/null; then
  bsdtar --format 7zip -cf test-books/manga.cb7 -C test-books/pages .
fi
if command -v rar >/dev/null; then
  (cd test-books/pages && rar a -r -idq ../manga.cbr .)
fi
python3 -c "
para = 'It was the best of times, it was the worst of times. ' * 6
open('test-books/novel.txt', 'w').write(''.join(f'{para}{i}\n\n' for i in range(3000)))"
# An EPUB novel: two chapters of writing and a picture between them.
python3 - <<'PY'
import zipfile
para = "<p>The lighthouse keeper climbed the stairs every evening, counting each step. </p>" * 40
z = zipfile.ZipFile("test-books/novel.epub", "w")
z.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
z.writestr("META-INF/container.xml", '<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>')
z.writestr("OEBPS/content.opf", '<package><manifest><item id="a" href="one.xhtml"/><item id="b" href="two.xhtml"/></manifest><spine><itemref idref="a"/><itemref idref="b"/></spine></package>')
z.writestr("OEBPS/one.xhtml", "<html><body><h1>One</h1>" + para + "<img src='map.png'/></body></html>")
z.writestr("OEBPS/two.xhtml", "<html><body><h1>Two</h1>" + para + "</body></html>")
z.write("test-books/pages/Chapter 1/page3.png", "OEBPS/map.png")
z.close()
PY
if [ "$(uname)" = Darwin ]; then
  sips -s format pdf "test-books/pages/Chapter 1/page1.png" --out test-books/page.pdf >/dev/null
  cupsfilter test-books/novel.txt > test-books/novel.pdf 2>/dev/null
  for f in rtf docx doc odt html; do
    textutil -convert $f test-books/novel.txt -output test-books/novel.$f
  done
fi
rm -rf test-books/pages
ls test-books
