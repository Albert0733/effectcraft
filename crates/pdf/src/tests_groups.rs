//! M13.12: transparency groups (isolated, knockout, group alpha) and soft masks on a group's
//! combined result.

use super::*;
use crate::tests::{page_pdf, render};

fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    (0..4).all(|i| (a[i] - b[i]).abs() < 0.02)
}

/// A page painting form `Fm` with graphics state `GS` set; the form has `group` as its
/// `/Group` entry body and `content`.
fn group_page(page: &str, gs: &str, group: &str, content: &str) -> Doc {
    let bytes = page_pdf(
        page,
        &format!("/ExtGState << /GS << {gs} >> /M << /BM /Multiply >> /H << /ca 0.5 >> >> /XObject << /Fm 10 0 R >>"),
        vec![(
            10,
            format!(
                "<< /Type /XObject /Subtype /Form /BBox [0 0 200 100] {group} /Resources << /ExtGState << /M << /BM /Multiply >> /H << /ca 0.5 >> >> >> >>"
            ),
            Some(content.as_bytes().to_vec()),
        )],
    );
    let doc = parse(&bytes).unwrap();
    assert!(doc.skipped.is_empty(), "{:?}", doc.skipped);
    doc
}

#[test]
fn group_alpha_applies_to_the_combined_result() {
    let content = "1 0 0 rg 0 0 100 100 re f 0 0 1 rg 50 0 100 100 re f";
    // A transparency group: the overlap is blue at half opacity (no red shows through).
    let doc = group_page("/GS gs /Fm Do", "/ca 0.5", "/Group << /S /Transparency >>", content);
    let img = render(&doc);
    assert!(near(img.get(75, 50), [0.0, 0.0, 0.5, 0.5]), "{:?}", img.get(75, 50));
    assert!(near(img.get(25, 50), [0.5, 0.0, 0.0, 0.5]), "{:?}", img.get(25, 50));
    // A plain form: each rectangle at half opacity.
    let doc = group_page("/GS gs /Fm Do", "/ca 0.5", "", content);
    let img = render(&doc);
    assert!(near(img.get(75, 50), [0.25, 0.0, 0.5, 0.75]), "{:?}", img.get(75, 50));
}

#[test]
fn knockout_groups_composite_with_the_initial_backdrop() {
    // Red, then half-opaque blue over it: knocked out, the overlap shows only the blue.
    let content = "1 0 0 rg 0 0 100 100 re f /H gs 0 0 1 rg 50 0 100 100 re f";
    let ko = group_page("/Fm Do", "", "/Group << /S /Transparency /I true /K true >>", content);
    let img = render(&ko);
    assert!(near(img.get(75, 50), [0.0, 0.0, 0.5, 0.5]), "knockout {:?}", img.get(75, 50));
    assert!(near(img.get(25, 50), [1.0, 0.0, 0.0, 1.0]), "{:?}", img.get(25, 50));
    let plain = group_page("/Fm Do", "", "/Group << /S /Transparency /I true >>", content);
    let img = render(&plain);
    assert!(near(img.get(75, 50), [0.5, 0.0, 0.5, 1.0]), "plain {:?}", img.get(75, 50));
    // Non-isolated knockout over a white page: the overlap is half blue over the page.
    let ko_bd = group_page("1 g 0 0 200 100 re f /Fm Do", "", "/Group << /S /Transparency /K true >>", content);
    let img = render(&ko_bd);
    assert!(near(img.get(75, 50), [0.5, 0.5, 1.0, 1.0]), "non-isolated knockout {:?}", img.get(75, 50));
}

#[test]
fn isolated_and_non_isolated_groups_with_blend_modes() {
    // Yellow page; the group multiplies cyan over it at half group opacity.
    let content = "/M gs 0 1 1 rg 0 0 200 100 re f";
    let non = group_page("1 1 0 rg 0 0 200 100 re f /GS gs /Fm Do", "/ca 0.5", "/Group << /S /Transparency >>", content);
    let img = render(&non);
    // yellow × cyan = green, half over yellow.
    assert!(near(img.get(100, 50), [0.5, 1.0, 0.0, 1.0]), "non-isolated {:?}", img.get(100, 50));
    let iso = group_page("1 1 0 rg 0 0 200 100 re f /GS gs /Fm Do", "/ca 0.5", "/Group << /S /Transparency /I true >>", content);
    let img = render(&iso);
    // Isolated: multiply against nothing = cyan, half over yellow.
    assert!(near(img.get(100, 50), [0.5, 1.0, 0.5, 1.0]), "isolated {:?}", img.get(100, 50));
}

#[test]
fn blend_modes_inside_clips_reach_the_backdrop() {
    let bytes = page_pdf("1 1 0 rg 0 0 200 100 re f q 0 0 100 100 re W n /M gs 0 1 1 rg 0 0 200 100 re f Q", "/ExtGState << /M << /BM /Multiply >> >>", vec![]);
    let img = render(&parse(&bytes).unwrap());
    assert!(near(img.get(50, 50), [0.0, 1.0, 0.0, 1.0]), "{:?}", img.get(50, 50));
    assert!(near(img.get(150, 50), [1.0, 1.0, 0.0, 1.0]), "{:?}", img.get(150, 50));
}

#[test]
fn soft_mask_applies_to_the_masked_group_result() {
    // Two overlapping opaque rectangles under a 50 % grey luminosity mask: the overlap is
    // the top rectangle at half opacity (the mask applies to their composite).
    let bytes = page_pdf(
        "q /GS1 gs 1 0 0 rg 0 0 100 100 re f 0 0 1 rg 50 0 100 100 re f Q",
        "/ExtGState << /GS1 << /SMask << /S /Luminosity /G 10 0 R >> >> >>",
        vec![(
            10,
            "<< /Type /XObject /Subtype /Form /BBox [0 0 200 100] /Group << /S /Transparency /CS /DeviceGray >> >>".into(),
            Some(b"0.5 g 0 0 200 100 re f".to_vec()),
        )],
    );
    let img = render(&parse(&bytes).unwrap());
    let p = img.get(75, 50);
    assert!((p[3] - 0.5).abs() < 0.02 && p[0] < 0.02 && (p[2] - 0.5).abs() < 0.02, "{p:?}");
}
