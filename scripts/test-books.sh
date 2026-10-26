#!/bin/bash
# Makes a book in every format this system's tools can write, in
# test-books/, for `leaf --check` to open: CBZ always; TAR and 7-Zip where
# bsdtar is; RAR where RARLAB's rar is; PDF, RTF and Word on macOS; and
# plain text. Usage: scripts/test-books.sh
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
if [ "$(uname)" = Darwin ]; then
  sips -s format pdf "test-books/pages/Chapter 1/page1.png" --out test-books/page.pdf >/dev/null
  textutil -convert rtf test-books/novel.txt -output test-books/novel.rtf
  textutil -convert docx test-books/novel.txt -output test-books/novel.docx
fi
rm -rf test-books/pages
ls test-books
