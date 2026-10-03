//! WebAssembly plug-ins: load, register, render through the effect registry, sandbox limits.

use effectcraft_effects::{Buf, EffectCtx, EffectEnv, Image};
use effectcraft_project::build::Ids;

use crate::load_wasm;

fn load_err(r: Result<&'static effectcraft_effects::EffectSpec, String>) -> String {
    match r {
        Err(e) => e,
        Ok(s) => panic!("unexpectedly loaded {}", s.id),
    }
}

/// A plug-in in WebAssembly text: multiplies RGB by `gain`, adds `lift` × `tint` when `on`.
fn gain_module(id: &str, render_body: &str) -> String {
    let manifest = format!(
        r#"{{"api":1,"id":"{id}","name":"Gain {id}","category":"Test Plug-ins","version":"1.0","params":[{{"id":"gain","name":"Gain","type":"slider","default":0.5,"min":0,"max":4}},{{"id":"on","name":"Lift","type":"checkbox","default":false}},{{"id":"tint","name":"Tint","type":"color","default":[0,0,1,1]}}]}}"#
    );
    let len = manifest.len();
    let escaped = manifest.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        r#"(module
  (memory (export "memory") 2)
  (data (i32.const 16) "{escaped}")
  (func (export "ec_api_version") (result i32) i32.const 1)
  (func (export "ec_manifest_ptr") (result i32) i32.const 16)
  (func (export "ec_manifest_len") (result i32) i32.const {len})
  (func (export "ec_alloc") (param $n i32) (result i32)
    (local $need i32)
    local.get $n
    i32.const 131071
    i32.add
    i32.const 16
    i32.shr_u
    memory.size
    i32.sub
    local.tee $need
    i32.const 0
    i32.gt_s
    if
      local.get $need
      memory.grow
      drop
    end
    i32.const 65536)
  (func (export "ec_render") (param $px i32) (param $w i32) (param $h i32) (param $params i32) (param $np i32) (param $t f64) (param $scale f64) (result i32)
    (local $i i32) (local $end i32) (local $g f32) (local $on f64)
    {render_body}
    local.get $params
    f64.load
    f32.demote_f64
    local.set $g
    local.get $params
    f64.load offset=8
    local.set $on
    local.get $px
    local.get $w
    local.get $h
    i32.mul
    i32.const 16
    i32.mul
    i32.add
    local.set $end
    local.get $px
    local.set $i
    block $done
      loop $l
        local.get $i
        local.get $end
        i32.ge_u
        br_if $done
        local.get $i
        local.get $i
        f32.load
        local.get $g
        f32.mul
        f32.store
        local.get $i
        local.get $i
        f32.load offset=4
        local.get $g
        f32.mul
        f32.store offset=4
        local.get $i
        local.get $i
        f32.load offset=8
        local.get $g
        f32.mul
        local.get $on
        local.get $params
        f64.load offset=32
        f64.mul
        f32.demote_f64
        f32.add
        f32.store offset=8
        local.get $i
        i32.const 16
        i32.add
        local.set $i
        br $l
      end
    end
    i32.const 0))"#
    )
}

fn render(spec: &effectcraft_effects::EffectSpec, gain: f64, on: bool) -> Image {
    let mut next = 1;
    let g = effectcraft_effects::instantiate(spec, &mut Ids(&mut next), spec.name, [4.0, 2.0]);
    let mut params = effectcraft_effects::flatten_params(&g, &mut |p| p.value.clone());
    params.values.insert("gain".into(), effectcraft_keyframe::Value::Scalar(gain));
    params.values.insert("on".into(), effectcraft_keyframe::Value::Bool(on));
    let ctx = EffectCtx { params: &params, time: 0.0, layer_size: [4.0, 2.0], seed: 1, adjustment: false, env: EffectEnv::default() };
    let buf = Buf { img: Image::filled(4, 2, [0.8, 0.6, 0.2, 1.0]), offset: [0.0; 2], scale: 1.0 };
    (spec.render)(&ctx, buf).img
}

#[test]
fn wasm_plugin_registers_and_renders() {
    let spec = load_wasm(gain_module("org.test.gain", "").as_bytes(), "gain.wat").unwrap();
    assert_eq!(spec.id, "org.test.gain");
    assert_eq!(spec.category, "Test Plug-ins");
    assert_eq!(spec.params.len(), 3);
    // Found like a built-in, listed under its own category.
    assert!(effectcraft_effects::find("org.test.gain").is_some());
    assert!(effectcraft_effects::lookup("Gain org.test.gain").is_some());
    assert!(effectcraft_effects::all().iter().any(|e| e.id == "org.test.gain"));
    assert!(effectcraft_effects::categories().contains(&"Test Plug-ins"));
    let img = render(spec, 0.5, false);
    assert_eq!(img.get(1, 1), [0.4, 0.3, 0.1, 1.0]);
    // The checkbox and colour parameters arrive flattened (tint blue = 1 → +1 on blue).
    let img = render(spec, 1.0, true);
    assert_eq!(img.get(3, 0), [0.8, 0.6, 1.2, 1.0]);
    // Deterministic, and safe to run on many threads at once (each takes an instance).
    let a: Vec<Image> = (0..8).map(|_| render(spec, 0.25, false)).collect();
    std::thread::scope(|sc| {
        let hs: Vec<_> = (0..4).map(|_| sc.spawn(|| render(spec, 0.25, false))).collect();
        for h in hs {
            assert_eq!(h.join().unwrap(), a[0]);
        }
    });
    assert!(a.iter().all(|i| *i == a[0]));
    // The same id can't be registered twice.
    assert!(load_err(load_wasm(gain_module("org.test.gain", "").as_bytes(), "again.wat")).contains("already registered"));
}

#[test]
fn wasm_plugins_are_sandboxed() {
    // A runaway loop runs out of fuel: the frame comes back unchanged instead of hanging.
    let spec = load_wasm(gain_module("org.test.spin", "(loop $spin br $spin)").as_bytes(), "spin.wat").unwrap();
    let img = render(spec, 0.5, false);
    assert_eq!(img.get(0, 0), [0.8, 0.6, 0.2, 1.0]);
    // No imports (files, clock, network…).
    let imports = r#"(module (import "env" "now" (func $now (result f64))) (memory (export "memory") 1))"#;
    assert!(load_err(load_wasm(imports.as_bytes(), "imports.wat")).contains("import"));
    // Garbage, wrong API version, bad manifests.
    assert!(load_wasm(b"\0asm nope", "bad.wasm").is_err());
    let v2 = gain_module("org.test.v2", "")
        .replace("(func (export \"ec_api_version\") (result i32) i32.const 1)", "(func (export \"ec_api_version\") (result i32) i32.const 2)");
    assert!(load_err(load_wasm(v2.as_bytes(), "v2.wat")).contains("API 2"));
    let reserved = gain_module("ec.blur.mine", "");
    assert!(load_err(load_wasm(reserved.as_bytes(), "reserved.wat")).contains("reserved"));
}
