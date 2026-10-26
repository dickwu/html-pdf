#!/usr/bin/env python3
"""Generate the synthetic PDFs used by tests/merge.php and tests/merge-matrix.php.

Everything here is made up: no real documents are read. Every page carries a
text marker "MARK-<fixture>-P<n>" so the merged output can be checked with
pdftotext.

Usage: generate.py OUT_DIR [--large]

Producers: hand-written PDF (inherited page-tree attributes, incremental
update), qpdf (object/xref streams, encryption), Ghostscript, ReportLab,
ImageMagick and macOS cupsfilter when they are installed; ironpress fixtures
are written by tests/merge-matrix.php itself. A missing tool skips its
fixtures and is listed in OUT_DIR/skipped.txt.
"""

import os
import shutil
import subprocess
import sys
import zlib

# ImageMagick has no default font on a bare Homebrew install; use the OFL
# font that already ships with the tests.
FONT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "MrsSaintDelafield-Regular.ttf")

OUT = sys.argv[1] if len(sys.argv) > 1 else "build"
LARGE = "--large" in sys.argv
os.makedirs(OUT, exist_ok=True)
skipped = []


def path(name):
    return os.path.join(OUT, name)


def have(tool):
    return shutil.which(tool) is not None


def run(*cmd, ok=(0,), **kw):
    res = subprocess.run(cmd, capture_output=True, **kw)
    if res.returncode not in ok:
        raise RuntimeError(f"{cmd[0]} failed ({res.returncode}): {res.stderr.decode()[:400]}")
    return res


# --------------------------------------------------------------------------
# Minimal raw PDF writer (classic xref), with incremental updates.


def text_stream(lines, x=72, y=720, size=18):
    ops = ["BT", f"/F1 {size} Tf", f"{x} {y} Td", f"{size + 6} TL"]
    for line in lines:
        ops.append("(" + line.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)") + ") Tj T*")
    ops.append("ET")
    return "\n".join(ops).encode()


def stream_obj(data, compress=True, extra=""):
    if compress:
        data = zlib.compress(data)
        extra += " /Filter /FlateDecode"
    return b"<< /Length %d%s >>\nstream\n" % (len(data), extra.encode()) + data + b"\nendstream"


def write_pdf(objects, root, version="1.4"):
    """objects: {num: bytes body}. Returns PDF bytes."""
    out = bytearray(b"%PDF-" + version.encode() + b"\n%\xe2\xe3\xcf\xd3\n")
    offsets = {}
    for num in sorted(objects):
        offsets[num] = len(out)
        out += b"%d 0 obj\n" % num + objects[num] + b"\nendobj\n"
    size = max(objects) + 1
    xref = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % size
    for num in range(1, size):
        if num in offsets:
            out += b"%010d 00000 n \n" % offsets[num]
        else:
            out += b"0000000000 65535 f \n"
    out += b"trailer\n<< /Size %d /Root %d 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (size, root, xref)
    return bytes(out), xref, size


def incremental_update(base, prev_xref, prev_size, objects, root):
    out = bytearray(base)
    offsets = {}
    for num in sorted(objects):
        offsets[num] = len(out)
        out += b"%d 0 obj\n" % num + objects[num] + b"\nendobj\n"
    xref = len(out)
    out += b"xref\n"
    for num in sorted(objects):
        out += b"%d 1\n%010d 00000 n \n" % (num, offsets[num])
    size = max(prev_size, max(objects) + 1)
    out += b"trailer\n<< /Size %d /Root %d 0 R /Prev %d >>\nstartxref\n%d\n%%%%EOF\n" % (
        size, root, prev_xref, xref)
    return bytes(out)


def inherited_tree():
    """3 pages under a nested page tree; Resources, MediaBox and Rotate live
    on the intermediate /Pages node and page 3 overrides MediaBox and Rotate."""
    objs = {
        1: b"<< /Type /Catalog /Pages 2 0 R >>",
        # The root carries a Rotate the pages must NOT pick up once merged
        # elsewhere -- and must keep while here (they override it below).
        2: b"<< /Type /Pages /Kids [3 0 R] /Count 3 >>",
        3: b"<< /Type /Pages /Parent 2 0 R /Kids [4 0 R 5 0 R 6 0 R] /Count 3"
           b" /Resources 7 0 R /MediaBox [0 0 420 595] /Rotate 90 >>",
        4: b"<< /Type /Page /Parent 3 0 R /Contents 9 0 R >>",
        5: b"<< /Type /Page /Parent 3 0 R /Contents 10 0 R >>",
        6: b"<< /Type /Page /Parent 3 0 R /Contents 11 0 R /MediaBox [0 0 612 792] /Rotate 0 >>",
        7: b"<< /Font << /F1 8 0 R >> >>",
        8: b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>",
    }
    for i, num in enumerate((9, 10, 11), start=1):
        objs[num] = stream_obj(text_stream([f"MARK-inherited-P{i}", "inherited Resources/MediaBox/Rotate"], y=500))
    return objs


def raw_fixtures():
    objs = inherited_tree()
    pdf, xref, size = write_pdf(objs, 1)
    open(path("raw-inherited.pdf"), "wb").write(pdf)

    # Incremental update: page 2's content replaced, a 4th page added.
    upd = {
        10: stream_obj(text_stream(["MARK-incremental-P2", "UPDATED in an incremental update"], y=500)),
        12: b"<< /Type /Page /Parent 3 0 R /Contents 13 0 R >>",
        13: stream_obj(text_stream(["MARK-incremental-P4", "page added by the update"], y=500)),
        3: b"<< /Type /Pages /Parent 2 0 R /Kids [4 0 R 5 0 R 6 0 R 12 0 R] /Count 4"
           b" /Resources 7 0 R /MediaBox [0 0 420 595] /Rotate 90 >>",
        2: b"<< /Type /Pages /Kids [3 0 R] /Count 4 >>",
    }
    inc = incremental_update(pdf, xref, size, upd, 1)
    open(path("raw-incremental.pdf"), "wb").write(inc)

    # Uncompressed, single page with an inline-dictionary Resources, a
    # 2x2 RGB image XObject and a link annotation pointing at its own page.
    img = bytes([255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0])
    objs = {
        1: b"<< /Type /Catalog /Pages 2 0 R >>",
        2: b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        3: b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R"
           b" /Resources << /Font << /F1 5 0 R >> /XObject << /Im1 6 0 R >> >>"
           b" /Annots [7 0 R] >>",
        4: stream_obj(text_stream(["MARK-rawimage-P1"]) + b"\nq 200 0 0 200 72 400 cm /Im1 Do Q", compress=False),
        5: b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        6: stream_obj(img, extra=" /Type /XObject /Subtype /Image /Width 2 /Height 2"
                                 " /ColorSpace /DeviceRGB /BitsPerComponent 8"),
        7: b"<< /Type /Annot /Subtype /Link /Rect [72 700 300 740] /Border [0 0 1]"
           b" /Dest [3 0 R /XYZ 0 842 0] >>",
    }
    pdf, _, _ = write_pdf(objs, 1)
    open(path("raw-image-annot.pdf"), "wb").write(pdf)

    # A valid file with an empty page tree.
    objs = {1: b"<< /Type /Catalog /Pages 2 0 R >>", 2: b"<< /Type /Pages /Kids [] /Count 0 >>"}
    pdf, _, _ = write_pdf(objs, 1)
    open(path("bad-no-pages.pdf"), "wb").write(pdf)


def bad_fixtures():
    good = open(path("raw-inherited.pdf"), "rb").read()
    open(path("bad-zero-bytes.pdf"), "wb").write(b"")
    open(path("bad-truncated-half.pdf"), "wb").write(good[: len(good) // 2])
    open(path("bad-truncated-tail.pdf"), "wb").write(good[:-60])  # xref table and trailer cut
    garbage = bytearray(good)
    mid = len(garbage) // 3
    garbage[mid: mid + 400] = bytes((i * 37) % 256 for i in range(400))
    open(path("bad-garbage-middle.pdf"), "wb").write(bytes(garbage))
    open(path("bad-header-only.pdf"), "wb").write(b"%PDF-1.7\n%%EOF\n")
    open(path("bad-html.pdf"), "wb").write(b"<!doctype html><html><body>not a pdf</body></html>")
    open(path("bad-png.pdf"), "wb").write(b"\x89PNG\r\n\x1a\n" + b"\0" * 64)
    open(path("bad-zip.pdf"), "wb").write(b"PK\x03\x04" + b"\0" * 64)


# --------------------------------------------------------------------------
# External producers.


def qpdf_fixtures():
    if not have("qpdf"):
        skipped.append("qpdf: object/xref streams and encrypted fixtures")
        return
    src = path("raw-inherited.pdf")
    run("qpdf", "--object-streams=generate", "--compress-streams=y", src, path("qpdf-objstm.pdf"))
    run("qpdf", "--encrypt", "user-pw", "owner-pw", "256", "--", src, path("enc-user-aes256.pdf"))
    run("qpdf", "--allow-weak-crypto", "--encrypt", "user-pw", "owner-pw", "40", "--", src, path("enc-user-rc4.pdf"),
        ok=(0, 3))
    run("qpdf", "--encrypt", "", "owner-pw", "256", "--modify=none", "--", src, path("enc-owner-aes256.pdf"))
    run("qpdf", "--encrypt", "", "owner-pw", "128", "--use-aes=y", "--", src, path("enc-owner-aes128.pdf"))


def gs_fixtures():
    if not have("gs"):
        skipped.append("gs: Ghostscript fixture")
        return
    ps = path("gs-source.ps")
    open(ps, "w").write("""%!PS
<< /PageSize [420 595] >> setpagedevice
/Times-Roman findfont 20 scalefont setfont
72 500 moveto (MARK-ghostscript-P1) show
0.2 0.4 0.8 setrgbcolor 72 300 200 120 rectfill showpage
<< /PageSize [842 595] >> setpagedevice
0 setgray /Helvetica-Bold findfont 20 scalefont setfont
72 500 moveto (MARK-ghostscript-P2 A4 landscape) show showpage
""")
    run("gs", "-q", "-dNOPAUSE", "-dBATCH", "-dSAFER", "-sDEVICE=pdfwrite",
        "-o", path("gs-mixed.pdf"), ps)
    os.remove(ps)


def magick_fixtures():
    if not have("magick"):
        skipped.append("magick: photo/scan fixtures")
        return
    jpg = path("photo.jpg")
    run("magick", "-size", "1200x900", "gradient:#3060c0-#f0c040", "-fill", "black", "-font", FONT,
        "-pointsize", "60", "-annotate", "+80+450", "MARK-magick-P1", "-quality", "85", jpg)
    run("magick", jpg, "-density", "150", "-units", "PixelsPerInch", path("magick-photo.pdf"))


def reportlab_fixtures():
    try:
        from reportlab.lib.pagesizes import A4, landscape, legal, letter
        from reportlab.pdfgen import canvas
    except ImportError:
        skipped.append("reportlab: Python reportlab not installed")
        return
    c = canvas.Canvas(path("reportlab-mixed.pdf"), pagesize=letter, invariant=1)
    sizes = [letter, legal, landscape(A4), A4]
    for i, size in enumerate(sizes, start=1):
        c.setPageSize(size)
        if i == 4:
            c.setPageRotation(90)
        c.setFont("Helvetica", 22)
        c.drawString(72, size[1] - 120, f"MARK-reportlab-P{i}")
        c.drawString(72, size[1] - 160, f"page size {size[0]:.0f}x{size[1]:.0f}")
        if os.path.exists(path("photo.jpg")):
            c.drawImage(path("photo.jpg"), 72, 100, width=240, height=180)
        c.showPage()
    c.save()

    if LARGE and have("magick"):
        # 60 pages, each with its own noisy photo so nothing deduplicates:
        # well over 20 MB.
        c = canvas.Canvas(path("large-60p.pdf"), pagesize=A4, invariant=1)
        # reportlab caches images by file name, so every page needs its own.
        tmps = []
        for i in range(1, 61):
            tmp = path(f"noise-{i}.jpg")
            tmps.append(tmp)
            run("magick", "-size", "1200x900", "-seed", str(i), "xc:gray", "+noise", "Random",
                "-quality", "92", tmp)
            c.setFont("Helvetica", 22)
            c.drawString(72, 790, f"MARK-large-P{i}")
            c.drawImage(tmp, 40, 80, width=515, height=680)
            c.showPage()
        c.save()
        for tmp in tmps:
            os.remove(tmp)


def cups_fixtures():
    if not have("cupsfilter"):
        skipped.append("cupsfilter: macOS Quartz fixture")
        return
    txt = path("cups.txt")
    open(txt, "w").write("MARK-cupsfilter-P1\nPrinted by the macOS CUPS text filter.\n")
    res = subprocess.run(["cupsfilter", "-i", "text/plain", "-m", "application/pdf", txt],
                         capture_output=True)
    if res.returncode != 0 or not res.stdout.startswith(b"%PDF"):
        skipped.append("cupsfilter: failed: " + res.stderr.decode()[:200])
    else:
        open(path("cups-text.pdf"), "wb").write(res.stdout)
    os.remove(txt)


raw_fixtures()
bad_fixtures()
qpdf_fixtures()
gs_fixtures()
magick_fixtures()
reportlab_fixtures()
cups_fixtures()
open(path("skipped.txt"), "w").write("\n".join(skipped) + ("\n" if skipped else ""))
for line in skipped:
    print("skipped:", line, file=sys.stderr)
