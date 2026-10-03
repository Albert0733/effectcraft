//! Project panel item edits: move into folders, rename, label, comment, with undo.

use effectcraft_project::ItemId;
use serde_json::json;

use crate::Session;

#[test]
fn move_into_folder_rename_label_comment_with_undo() {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "A", "width": 64, "height": 64, "frameRate": 24, "duration": 1})).unwrap();
    let comp = s.active_comp_id().unwrap();
    let f = ItemId(s.execute("project.newFolder", json!({"name": "Shots"})).unwrap()["item"].as_u64().unwrap());
    let g = ItemId(s.execute("project.newFolder", json!({"name": "Inner", "parent": f.0})).unwrap()["item"].as_u64().unwrap());

    s.execute("project.move", json!({"items": [comp.0], "folder": "Shots"})).unwrap();
    assert_eq!(s.project.item(comp).unwrap().parent, Some(f));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project.item(comp).unwrap().parent, None);
    s.execute("edit.redo", json!({})).unwrap();
    // Out of the folder (to the root) via the selection.
    s.execute("project.select", json!({"items": [comp.0]})).unwrap();
    s.execute("project.move", json!({"folder": null})).unwrap();
    assert_eq!(s.project.item(comp).unwrap().parent, None);
    // A folder can't go into itself or its descendants.
    assert!(s.execute("project.move", json!({"items": [f.0], "folder": g.0})).is_err());
    assert!(s.execute("project.move", json!({"items": [comp.0], "folder": comp.0})).is_err());

    s.execute("project.rename", json!({"item": comp.0, "name": "Hero"})).unwrap();
    assert_eq!(s.project.item(comp).unwrap().name, "Hero");
    assert!(s.execute("project.rename", json!({"item": comp.0, "name": "  "})).is_err());
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project.item(comp).unwrap().name, "A");

    s.execute("project.setLabel", json!({"items": [comp.0], "label": "Pink"})).unwrap();
    assert_eq!(s.project.item(comp).unwrap().label.name(), "Pink");
    s.execute("project.setComment", json!({"item": comp.0, "comment": "final"})).unwrap();
    assert_eq!(s.project.item(comp).unwrap().comment, "final");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project.item(comp).unwrap().comment, "");
}
