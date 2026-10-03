//! Test files are generated here (no third-party samples).

use super::*;

use crate::write::pdf;

const CONTENT: &str = "/OC /MC0 BDC\n0 1 0 rg 0 0 200 100 re f\nEMC\n\
/OC /MC1 BDC\nq 10 10 80 80 re W n\n1 0 0 rg 0 0 50 100 re f\nQ\n\
/Pattern cs /P0 scn 100 0 100 50 re f\n\
0 0 1 RG 4 w 120 80 m 180 80 l S\n\
BT /F1 12 Tf (ignored) Tj ET\nEMC\n";

pub(crate) fn sample_pdf(object_stream: bool) -> Vec<u8> {
    let page = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Resources << /Properties << /MC0 5 0 R /MC1 6 0 R >> /Pattern << /P0 7 0 R >> >> /Contents 4 0 R >>";
    let pattern = "<< /PatternType 2 /Shading << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [100 0 200 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> /Extend [true true] >> >>";
    let mut objs = vec![
        (1, "<< /Type /Catalog /Pages 2 0 R >>".to_string(), None),
        (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(), None),
        (3, page.to_string(), None),
        (4, "<< >>".to_string(), Some(CONTENT.as_bytes().to_vec())),
        (7, pattern.to_string(), None),
    ];
    let ocg5 = "<< /Type /OCG /Name (Background) >>";
    let ocg6 = "<< /Type /OCG /Name <FEFF00410072007400> >>"; // "Art" in UTF-16BE
    if object_stream {
        let body = format!("{ocg5} {ocg6}");
        let head = format!("5 0 6 {} ", ocg5.len() + 1);
        let data = format!("{head}{body}");
        objs.push((8, format!("<< /Type /ObjStm /N 2 /First {} >>", head.len()), Some(data.into_bytes())));
    } else {
        objs.push((5, ocg5.to_string(), None));
        objs.push((6, ocg6.to_string(), None));
    }
    pdf(&objs, 1)
}

fn px(doc: &Doc, x: i64, y: i64) -> [f32; 4] {
    let (w, h) = doc.pixel_size();
    let img = effectcraft_svg::rasterize(doc, w, h, 1.0);
    img.get(x, y)
}

#[test]
fn pdf_paths_clip_gradient_and_layers() {
    for objstm in [false, true] {
        let bytes = sample_pdf(objstm);
        assert_eq!(sniff(&bytes), Some(Format::Pdf));
        assert_eq!(page_count(&bytes), 1);
        let doc = parse(&bytes).unwrap();
        assert_eq!((doc.width, doc.height), (200.0, 100.0));
        assert_eq!(layer_names(&doc), vec!["Background", "Art"], "object stream {objstm}");
        assert!(doc.skipped.contains(&"text".to_string()));
        let (w, h) = doc.pixel_size();
        let img = effectcraft_svg::rasterize(&doc, w, h, 1.0);
        let at = |x: i64, y: i64| img.get(x, y);
        // Red rectangle clipped to x ≥ 10, over the green background.
        assert!(at(30, 50)[0] > 0.99 && at(30, 50)[1] < 0.01, "{:?}", at(30, 50));
        assert!(at(5, 50)[1] > 0.99 && at(5, 50)[0] < 0.01, "clipped out: {:?}", at(5, 50));
        assert!(at(70, 50)[1] > 0.99, "{:?}", at(70, 50));
        // Axial shading, red → blue left to right (in the lower half: PDF y is up).
        let (l, r) = (at(110, 75), at(190, 75));
        assert!(l[0] > 0.8 && l[2] < 0.2 && r[2] > 0.8 && r[0] < 0.2, "{l:?} {r:?}");
        assert!(at(150, 25)[1] > 0.99, "the gradient stays in its rectangle: {:?}", at(150, 25));
        // Stroked line (PDF y = 80 → 20 px from the top).
        assert!(at(150, 20)[2] > 0.99 && at(150, 20)[1] < 0.01, "{:?}", at(150, 20));
        // One layer at a time.
        let bg = layer_doc(&doc, 0);
        assert!(px(&bg, 30, 50)[1] > 0.99);
        let art = layer_doc(&doc, 1);
        assert_eq!(px(&art, 5, 50)[3], 0.0);
        // Continuous rasterisation: twice the size stays sharp at the clip edge.
        let big = effectcraft_svg::rasterize(&doc, 400, 200, 2.0);
        assert!(big.get(21, 100)[0] > 0.99 && big.get(18, 100)[1] > 0.99);
    }
}

#[test]
fn pdf_forms_rotation_cmyk_and_alpha() {
    let content = "q 2 0 0 2 0 0 cm /Fm0 Do Q\n/GS0 gs 0 0 0 1 k 0 0 10 10 re f\n";
    let form = "0 1 1 0 k 0 0 20 20 re f";
    let objs = vec![
        (1, "<< /Type /Catalog /Pages 2 0 R >>".to_string(), None),
        (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 /Rotate 90 >>".to_string(), None),
        (
            3,
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 50] /Resources << /XObject << /Fm0 5 0 R >> /ExtGState << /GS0 << /ca 0.5 >> >> >> /Contents 4 0 R >>"
                .to_string(),
            None,
        ),
        (4, "<< >>".to_string(), Some(content.as_bytes().to_vec())),
        (5, "<< /Type /XObject /Subtype /Form /BBox [0 0 10 10] >>".to_string(), Some(form.as_bytes().to_vec())),
    ];
    let doc = parse(&pdf(&objs, 1)).unwrap();
    // Rotated a quarter turn: 50 × 100.
    assert_eq!((doc.width, doc.height), (50.0, 100.0));
    let (w, h) = doc.pixel_size();
    let img = effectcraft_svg::rasterize(&doc, w, h, 1.0);
    // The form (CMYK red, clipped to its 10×10 box, scaled ×2) covers PDF (0..20, 0..20); the
    // half-transparent black square covers PDF (0..10, 0..10). /Rotate 90: PDF (x, y) → (y, x).
    let red = img.get(15, 15);
    assert!(red[0] > 0.99 && red[1] < 0.01 && red[3] > 0.99, "{red:?}");
    let dark = img.get(5, 5);
    assert!((dark[0] - 0.5).abs() < 0.02 && dark[3] > 0.99, "{dark:?}");
    assert_eq!(img.get(30, 30)[3], 0.0, "form clipped to its bounding box");
}

const EPS: &str = "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n\
/m {moveto} bind def /l {lineto} bind def\n\
/cm { 6 array astore concat } bind def\n\
/box { 4 dict begin /h exch def /w exch def /y exch def /x exch def x y m w 0 rlineto 0 h rlineto w neg 0 rlineto closepath end } bind def\n\
1 0 0 setrgbcolor\n10 10 30 30 box fill\n\
gsave 0 0 1 setrgbcolor 70 50 20 0 360 arc fill grestore\n\
gsave 1 0 0 1 50 0 cm 0 1 0 setrgbcolor 0 0 10 10 box fill grestore\n\
0 setgray 2 setlinewidth 0 90 m 100 90 l stroke\n\
/Helvetica findfont 12 scalefont setfont 5 5 moveto (text) show\n\
showpage\n%%EOF\n";

#[test]
fn eps_postscript_subset() {
    let doc = parse(EPS.as_bytes()).unwrap();
    assert_eq!((doc.width, doc.height), (100.0, 100.0));
    assert!(doc.skipped.contains(&"text".to_string()));
    let at = |x, y| px(&doc, x, y);
    assert!(at(25, 75)[0] > 0.99 && at(25, 75)[3] > 0.99, "red box {:?}", at(25, 75));
    assert!(at(70, 50)[2] > 0.99, "blue disc {:?}", at(70, 50));
    assert!(at(55, 95)[1] > 0.99, "translated green box {:?}", at(55, 95));
    let line = at(50, 10);
    assert!(line[3] > 0.99 && line[0] < 0.01, "black line {line:?}");
    assert_eq!(at(95, 70)[3], 0.0);
    // DOS EPS binary header (with a fake TIFF preview after the PostScript).
    let ps = EPS.as_bytes();
    let mut dos = vec![0xC5, 0xD0, 0xD3, 0xC6];
    dos.extend_from_slice(&30u32.to_le_bytes());
    dos.extend_from_slice(&(ps.len() as u32).to_le_bytes());
    dos.extend_from_slice(&[0; 18]);
    dos.extend_from_slice(ps);
    dos.extend_from_slice(b"II*\0 not a real preview");
    let d2 = parse(&dos).unwrap();
    assert_eq!(d2.root.children.len(), doc.root.children.len());
    assert_eq!(codec("art.eps", &dos), Some("EPS"));
    assert_eq!(codec("art.ai", &sample_pdf(false)), Some("AI"));
    assert_eq!(parse(b"hello"), Err(Error::NotVector));
}

#[test]
fn filters_decode() {
    assert_eq!(object::ascii85(b"<~87cURD]i,\"Ebo80~>"), b"Hello World!".to_vec());
    let z = miniz_oxide::deflate::compress_to_vec_zlib(b"abc", 6);
    assert_eq!(object::inflate(&z).unwrap(), b"abc");
}
