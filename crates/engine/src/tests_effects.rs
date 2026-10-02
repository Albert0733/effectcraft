//! Effect Controls operations: copy/paste, duplicate, reorder, remove (Delete), reset, and the
//! Edit menu routing to them when effects are selected, all with undo.

use effectcraft_keyframe::Value as KV;
use effectcraft_project::{Layer, LayerId};
use serde_json::json;

use crate::Session;

fn comp_with_two_solids() -> (Session, u64, u64) {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "Main", "width": 640, "height": 360, "frameRate": 30, "duration": 10})).unwrap();
    let a = s.execute("layer.newSolid", json!({"color": "#ff0000", "width": 200, "height": 100})).unwrap()["layer"].as_u64().unwrap();
    let b = s.execute("layer.newSolid", json!({"color": "#00ff00", "width": 400, "height": 300})).unwrap()["layer"].as_u64().unwrap();
    (s, a, b)
}

fn layer(s: &Session, id: u64) -> Layer {
    s.active_comp().unwrap().layer(LayerId(id)).unwrap().clone()
}

fn fx_names(s: &Session, id: u64) -> Vec<String> {
    layer(s, id).effects().map(|f| f.groups().map(|g| g.name.clone()).collect()).unwrap_or_default()
}

fn apply(s: &mut Session, lid: u64, effect: &str) -> u64 {
    s.execute("effect.apply", json!({"layers": [lid], "effect": effect})).unwrap()["effects"][0].as_u64().unwrap()
}

fn select_effect(s: &mut Session, lid: u64, uid: u64) {
    s.state.selected_layers = vec![LayerId(lid)];
    s.state.selected_props = vec![(LayerId(lid), uid)];
}

#[test]
fn copy_paste_effects_between_layers_with_undo() {
    let (mut s, a, b) = comp_with_two_solids();
    let blur = apply(&mut s, a, "Gaussian Blur");
    let gid = layer(&s, a).effects().unwrap().groups().next().unwrap().get("blurriness").unwrap().uid;
    s.execute("prop.set", json!({"layer": a, "prop": gid, "value": 12.5})).unwrap();
    select_effect(&mut s, a, blur);
    // Edit ▸ Copy with an effect selected copies the effect, not the layer.
    s.execute("edit.copy", json!({})).unwrap();
    assert_eq!(s.state.effect_clipboard.len(), 1);
    assert!(s.state.clipboard.is_empty());
    // Paste onto the other layer.
    s.state.selected_layers = vec![LayerId(b)];
    s.state.selected_props.clear();
    let n_layers = s.active_comp().unwrap().layers.len();
    s.execute("edit.paste", json!({})).unwrap();
    assert_eq!(s.active_comp().unwrap().layers.len(), n_layers, "no layer pasted");
    assert_eq!(fx_names(&s, b), vec!["Gaussian Blur"]);
    let pasted = layer(&s, b).effects().unwrap().groups().next().unwrap().clone();
    assert_ne!(pasted.uid, blur, "fresh uids");
    assert_eq!(pasted.get("blurriness").unwrap().value, KV::Scalar(12.5));
    // Pasting again names the second instance uniquely.
    s.execute("effect.paste", json!({"layers": [b]})).unwrap();
    assert_eq!(fx_names(&s, b), vec!["Gaussian Blur", "Gaussian Blur 2"]);
    s.undo();
    s.undo();
    assert!(fx_names(&s, b).is_empty());
    assert_eq!(fx_names(&s, a), vec!["Gaussian Blur"]);
}

#[test]
fn duplicate_reorder_and_delete_selected_effect() {
    let (mut s, a, _) = comp_with_two_solids();
    let blur = apply(&mut s, a, "Gaussian Blur");
    apply(&mut s, a, "Invert");
    select_effect(&mut s, a, blur);
    // Edit ▸ Duplicate (Cmd+D) duplicates the selected effect right after it and selects the copy.
    s.execute("edit.duplicate", json!({})).unwrap();
    assert_eq!(fx_names(&s, a), vec!["Gaussian Blur", "Gaussian Blur 2", "Invert"]);
    let dup = s.state.selected_props[0].1;
    assert_ne!(dup, blur);
    // Reorder: move Invert to the top.
    s.execute("effect.reorder", json!({"layer": a, "effect": "Invert", "index": 1})).unwrap();
    assert_eq!(fx_names(&s, a), vec!["Invert", "Gaussian Blur", "Gaussian Blur 2"]);
    // Delete with the duplicate selected removes just that effect (not the layer).
    s.execute("edit.clear", json!({})).unwrap();
    assert_eq!(fx_names(&s, a), vec!["Invert", "Gaussian Blur"]);
    assert!(s.active_comp().unwrap().layer(LayerId(a)).is_some());
    s.undo();
    assert_eq!(fx_names(&s, a), vec!["Invert", "Gaussian Blur", "Gaussian Blur 2"]);
    s.undo();
    assert_eq!(fx_names(&s, a), vec!["Gaussian Blur", "Gaussian Blur 2", "Invert"]);
    s.undo();
    assert_eq!(fx_names(&s, a), vec!["Gaussian Blur", "Invert"]);
}

#[test]
fn reset_effect_restores_defaults_in_one_undo_step() {
    let (mut s, _, b) = comp_with_two_solids();
    let ramp = apply(&mut s, b, "Gradient Ramp");
    let g = layer(&s, b).effects().unwrap().groups().next().unwrap().clone();
    let start = g.get("start").unwrap().uid;
    let shape = g.get("shape").unwrap().uid;
    // Point defaults are fractions of the layer (400 × 300).
    assert_eq!(g.get("start").unwrap().value, KV::Vec2([200.0, 0.0]));
    s.execute("prop.set", json!({"layer": b, "prop": start, "value": [10.0, 20.0]})).unwrap();
    s.execute("prop.set", json!({"layer": b, "prop": shape, "value": 1})).unwrap();
    let undo_before = s.history.undo.len();
    s.execute("effect.reset", json!({"layer": b, "effect": ramp})).unwrap();
    assert_eq!(s.history.undo.len(), undo_before + 1);
    let g = layer(&s, b).effects().unwrap().groups().next().unwrap().clone();
    assert_eq!(g.get("start").unwrap().value, KV::Vec2([200.0, 0.0]));
    assert_eq!(g.get("shape").unwrap().value, KV::Enum(0));
    s.undo();
    let g = layer(&s, b).effects().unwrap().groups().next().unwrap().clone();
    assert_eq!(g.get("start").unwrap().value, KV::Vec2([10.0, 20.0]));
}

#[test]
fn reset_keys_animated_params_at_the_cti() {
    let (mut s, a, _) = comp_with_two_solids();
    let blur = apply(&mut s, a, "Gaussian Blur");
    let uid = layer(&s, a).effects().unwrap().groups().next().unwrap().get("blurriness").unwrap().uid;
    s.execute("prop.toggleAnimation", json!({"layer": a, "prop": uid})).unwrap();
    s.execute("prop.set", json!({"layer": a, "prop": uid, "value": 30.0})).unwrap();
    s.execute("time.set", json!({"time": 2.0})).unwrap();
    s.execute("effect.reset", json!({"layer": a, "effect": blur})).unwrap();
    let pr = layer(&s, a).effects().unwrap().groups().next().unwrap().get("blurriness").unwrap().clone();
    assert_eq!(pr.keys.len(), 2, "a key at the CTI with the default value");
    assert_eq!(pr.keys[1].value, KV::Scalar(0.0));
}

#[test]
fn copy_layers_clears_effect_clipboard() {
    let (mut s, a, b) = comp_with_two_solids();
    let blur = apply(&mut s, a, "Gaussian Blur");
    select_effect(&mut s, a, blur);
    s.execute("effect.copy", json!({})).unwrap();
    s.state.selected_props.clear();
    s.state.selected_layers = vec![LayerId(b)];
    s.execute("edit.copy", json!({})).unwrap();
    assert!(s.state.effect_clipboard.is_empty());
    let n = s.active_comp().unwrap().layers.len();
    s.execute("edit.paste", json!({})).unwrap();
    assert_eq!(s.active_comp().unwrap().layers.len(), n + 1, "layer pasted");
}

#[test]
fn layer_params_get_a_source_companion() {
    let (mut s, a, _) = comp_with_two_solids();
    apply(&mut s, a, "ec.channel.blend");
    let g = layer(&s, a).effects().unwrap().groups().next().unwrap().clone();
    let layer_params: Vec<_> = g.props().filter(|p| matches!(p.ui, effectcraft_project::ParamUi::Layer)).map(|p| p.match_id.clone()).collect();
    assert!(!layer_params.is_empty());
    for id in layer_params {
        let src = g.get(&effectcraft_effects::layer_source_id(&id)).expect("companion");
        assert_eq!(src.value, KV::Enum(2), "Effects & Masks by default");
        assert!(matches!(src.ui, effectcraft_project::ParamUi::Hidden));
    }
}
