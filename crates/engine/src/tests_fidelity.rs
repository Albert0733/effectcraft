//! Render-fidelity commands: project colour settings, footage colour profile, slip edit.

use effectcraft_project::{BitDepth, ColorSpace};
use serde_json::json;

use crate::Session;

#[test]
fn project_colour_settings_are_undoable() {
    let mut s = Session::default();
    s.execute("file.projectSettings", json!({"bitDepth": 32, "workingSpace": "rec2020", "linearize": true, "blendLinear": true})).unwrap();
    let st = &s.project.settings;
    assert_eq!((st.bit_depth, st.working_space, st.linearize, st.blend_linear), (BitDepth::Bpc32, Some(ColorSpace::Rec2020), true, true));
    s.execute("file.projectSettings", json!({"workingSpace": "Display P3"})).unwrap();
    assert_eq!(s.project.settings.working_space, Some(ColorSpace::DisplayP3));
    s.execute("file.projectSettings", json!({"workingSpace": "none"})).unwrap();
    assert_eq!(s.project.settings.working_space, None);
    assert!(s.execute("file.projectSettings", json!({"workingSpace": "acescg-ish"})).is_err());
    assert!(s.undo());
    assert_eq!(s.project.settings.working_space, Some(ColorSpace::DisplayP3));
    assert!(s.undo() && s.undo());
    assert_eq!(s.project.settings.bit_depth, BitDepth::Bpc8);
    assert!(!s.project.settings.blend_linear);
    // The Project panel's depth button cycles 8 → 16 → 32 → 8.
    for want in ["16 bpc", "32 bpc", "8 bpc"] {
        assert_eq!(s.execute("file.cycleBitDepth", json!({})).unwrap(), json!(want));
    }
}
