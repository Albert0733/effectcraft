//! M13.12: CCITT images in PDF pages, EPS text and parser robustness over truncated and
//! corrupt inputs (generated fixtures).

use super::*;
use crate::tests::{page_pdf, render};

/// An image XObject of `img` (black = true) CCITT-encoded with `k`, drawn over the page.
fn ccitt_page(img: &[Vec<bool>], k: i64, mask: bool) -> Doc {
    let (w, h) = (img[0].len(), img.len());
    let data = crate::ccitt::tests::encode(img, k);
    let kind = if mask { "/ImageMask true" } else { "/ColorSpace /DeviceGray /BitsPerComponent 1" };
    let bytes = page_pdf(
        "0 0 1 rg q 200 0 0 100 0 0 cm /Im Do Q",
        "/XObject << /Im 10 0 R >>",
        vec![(
            10,
            format!(
                "<< /Type /XObject /Subtype /Image /Width {w} /Height {h} {kind} /Filter /CCITTFaxDecode /DecodeParms << /K {k} /Columns {w} /Rows {h} /EndOfLine {} >> >>",
                k >= 0
            ),
            Some(data),
        )],
    );
    let doc = parse(&bytes).unwrap();
    assert!(doc.skipped.is_empty(), "{:?}", doc.skipped);
    doc
}

#[test]
fn ccitt_images_and_stencil_masks() {
    // Left half black, a white bar on rows 4..6 (20×10 pixels → 10 PDF units per pixel).
    let img: Vec<Vec<bool>> = (0..10).map(|y| (0..20).map(|x| x < 10 && !(4..6).contains(&y)).collect()).collect();
    for k in [-1, 0, 2] {
        let doc = ccitt_page(&img, k, false);
        let r = render(&doc);
        let px = |x: usize, y: usize| r.get(x as i64 * 10 + 5, y as i64 * 10 + 5);
        assert!(px(2, 2)[0] < 0.01 && px(2, 2)[3] > 0.99, "K {k}: black {:?}", px(2, 2));
        assert!(px(15, 2)[0] > 0.99, "K {k}: white {:?}", px(15, 2));
        assert!(px(2, 4)[0] > 0.99, "K {k}: the white bar {:?}", px(2, 4));
        // Stencil mask: black (sample 0) pixels paint the fill colour, the rest is clear.
        let doc = ccitt_page(&img, k, true);
        let r = render(&doc);
        let px = |x: usize, y: usize| r.get(x as i64 * 10 + 5, y as i64 * 10 + 5);
        assert!(px(2, 2)[2] > 0.99 && px(2, 2)[3] > 0.99, "K {k}: mask {:?}", px(2, 2));
        assert_eq!(px(15, 2)[3], 0.0, "K {k}");
    }
}

/// Byte-level mutations of `bytes`: truncations, flipped bytes, deleted and duplicated
/// spans (deterministic).
fn mutations(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut out = vec![];
    let n = bytes.len();
    for k in 1..24 {
        out.push(bytes[..n * k / 24].to_vec());
    }
    let mut s = 0x9E37_79B9u32;
    let mut rnd = move || {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        s
    };
    for _ in 0..60 {
        let mut b = bytes.to_vec();
        for _ in 0..1 + rnd() % 6 {
            let i = rnd() as usize % n;
            b[i] = match rnd() % 4 {
                0 => b[i] ^ 0xFF,
                1 => b'0' + (rnd() % 10) as u8,
                2 => b"[]<>()/{}% \n"[(rnd() % 12) as usize],
                _ => rnd() as u8,
            };
        }
        out.push(b);
    }
    for _ in 0..30 {
        let mut b = bytes.to_vec();
        let i = rnd() as usize % n;
        let len = (rnd() as usize % 64).min(n - i);
        if rnd() % 2 == 0 {
            b.drain(i..i + len);
        } else {
            let span = b[i..i + len].to_vec();
            b.splice(i..i, span);
        }
        out.push(b);
    }
    out
}

/// Parse and render every page of every mutation: errors are fine, panics are not.
fn survive(name: &str, bytes: &[u8]) {
    for (i, m) in mutations(bytes).into_iter().enumerate() {
        let r = std::panic::catch_unwind(|| {
            let pages = page_count(&m);
            for p in 0..pages.clamp(1, 3) {
                if let Ok(doc) = parse_page(&m, p) {
                    let (w, h) = doc.pixel_size();
                    let s = 64.0 / (w.max(h).max(1) as f64);
                    let _ = effectcraft_svg::rasterize(&doc, 64, 64, s);
                    let _ = layer_names(&doc);
                }
            }
        });
        assert!(r.is_ok(), "{name}: mutation {i} panicked");
    }
}

#[test]
fn truncated_and_corrupt_pdfs_never_panic() {
    survive("sample", &crate::tests::sample_pdf(false));
    survive("object streams", &crate::tests::sample_pdf(true));
    let mut fonts = crate::tests::square_font_objs();
    fonts.push((13, "<< /Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 >>".into(), Some(vec![255; 12])));
    survive(
        "text and images",
        &page_pdf("BT /F1 40 Tf 10 10 Td (AAA) Tj ET q 50 0 0 50 0 0 cm /Im Do Q", "/Font << /F1 10 0 R >> /XObject << /Im 13 0 R >>", fonts),
    );
    let img: Vec<Vec<bool>> = (0..8).map(|y| (0..16).map(|x| (x + y) % 3 == 0).collect()).collect();
    let data = crate::ccitt::tests::encode(&img, -1);
    survive(
        "ccitt",
        &page_pdf(
            "q 100 0 0 50 0 0 cm /Im Do Q",
            "/XObject << /Im 10 0 R >>",
            vec![(
                10,
                "<< /Type /XObject /Subtype /Image /Width 16 /Height 8 /ImageMask true /Filter /CCITTFaxDecode /DecodeParms << /K -1 /Columns 16 >> >>".into(),
                Some(data),
            )],
        ),
    );
}

#[test]
fn hostile_structures_return_errors_or_draw_nothing() {
    // Not a PDF / EPS at all, empty, and headers alone.
    for b in [&b""[..], b"%PDF-", b"%PDF-1.7\n%%EOF", b"%!PS-Adobe-3.0 EPSF-3.0\n", b"\xC5\xD0\xD3\xC6", b"hello"] {
        let _ = parse(b);
        let _ = page_count(b);
    }
    assert_eq!(parse(b"hello"), Err(Error::NotVector));
    assert_eq!(parse(b"%PDF-1.7\n%%EOF"), Err(Error::NoPages));
    // A page tree that refers to itself, a content stream that is a reference loop, deep
    // form recursion and absurd numbers.
    let objs: crate::tests::Objs = vec![
        (1, "<< /Type /Catalog /Pages 2 0 R >>".into(), None),
        (2, "<< /Type /Pages /Kids [2 0 R 3 0 R] /Count 1 >>".into(), None),
        (3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1e300 -1e300] /Contents 5 0 R /Resources << /XObject << /F 4 0 R >> >> >>".into(), None),
        (
            4,
            "<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /F 4 0 R >> >> >>".into(),
            Some(b"/F Do 1e308 1e308 m 1e308 0 l f".to_vec()),
        ),
        (5, "<< >>".into(), Some(b"/F Do q q q q Q Q Q Q Q Q 99999999999 w [1 0 0 1 0 0] cm".to_vec())),
    ];
    let bytes = crate::write::pdf(&objs, 1);
    if let Ok(doc) = parse(&bytes) {
        let _ = effectcraft_svg::rasterize(&doc, 32, 32, 1e-300);
    }
    // Images with absurd sizes and filters.
    let bytes = page_pdf(
        "/A Do /B Do /C Do /D Do",
        "/XObject << /A 10 0 R /B 11 0 R /C 12 0 R /D 13 0 R >>",
        vec![
            (10, "<< /Subtype /Image /Width 100000 /Height 100000 /BitsPerComponent 8 /ColorSpace /DeviceRGB >>".into(), Some(vec![0; 4])),
            (11, "<< /Subtype /Image /Width 4 /Height 4 /BitsPerComponent 7 /ColorSpace /DeviceRGB >>".into(), Some(vec![0; 4])),
            (
                12,
                "<< /Subtype /Image /Width 4 /Height 4 /BitsPerComponent 1 /ImageMask true /Filter /CCITTFaxDecode /DecodeParms << /K -1 /Columns 0 >> >>"
                    .into(),
                Some(vec![0xFF; 4]),
            ),
            (13, "<< /Subtype /Image /Width 4 /Height 4 /BitsPerComponent 8 /ColorSpace [/Indexed /DeviceRGB 300 <00>] >>".into(), Some(vec![9; 16])),
        ],
    );
    let doc = parse(&bytes).unwrap();
    let _ = render(&doc);
    assert!(!doc.skipped.is_empty());
}

#[test]
fn truncated_and_corrupt_eps_never_panic() {
    let eps = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n\
/sq { newpath 0 0 moveto 10 0 lineto 10 10 lineto closepath } bind def\n\
1 0 0 setrgbcolor 0 1 9 { gsave dup 10 mul 0 translate sq fill grestore } for\n\
/Helvetica findfont 12 scalefont setfont 10 50 moveto (Hello) show\n\
[1 2] 0 setdash 0 0 moveto 100 100 lineto stroke\n\
1 1 1 { pop } repeat { exit } loop 0 0 50 0 360 arc closepath eofill\n%%EOF\n";
    survive("eps", eps);
    // Stack underflow, unbalanced procedures, runaway loops and huge repeats.
    for prog in [
        &b"pop pop pop add mul def"[..],
        b"{ { { { {",
        b"} } ] >> )",
        b"{ } loop",
        b"0 1 1e9 { pop } for",
        b"1e9 { 1 } repeat",
        b"/a { a } def a",
        b"(unterminated",
        b"<abc",
        b"100 100 scale 0 0 moveto 1e308 1e308 lineto stroke",
    ] {
        let mut b = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 10 10\n".to_vec();
        b.extend_from_slice(prog);
        let r = std::panic::catch_unwind(|| {
            if let Ok(doc) = parse(&b) {
                let _ = effectcraft_svg::rasterize(&doc, 10, 10, 1.0);
            }
        });
        assert!(r.is_ok(), "{}", String::from_utf8_lossy(prog));
    }
}
