//! Property evaluation (keyframes + expressions) and layer transforms.
//!
//! Keyframe times are stored in **layer time** (they move with the layer), so evaluating a
//! property at comp time `t` first maps `t` through the layer's start time and stretch.

use effectcraft_geom::{Mat3, Mat4, Vec3, look_at, vec3};
use effectcraft_keyframe::Value;
use effectcraft_project::{Comp, ItemId, Layer, LayerId, LayerSource, Project, PropGroup, Property};
use effectcraft_time::Tick;

/// Expression evaluation hook (implemented by `effectcraft-expr`).
pub trait ExprHost: Send + Sync {
    /// Evaluate the expression of `prop` (on `layer`, comp time `t`), given the keyframed value.
    fn eval(&self, ctx: &EvalCtx, layer: &Layer, prop: &Property, value: &Value) -> Result<Value, String>;
}

#[derive(Clone, Copy)]
pub struct EvalCtx<'a> {
    pub project: &'a Project,
    pub comp_id: ItemId,
    pub comp: &'a Comp,
    /// Comp time.
    pub time: Tick,
    pub expr: Option<&'a dyn ExprHost>,
}

impl<'a> EvalCtx<'a> {
    pub fn new(project: &'a Project, comp_id: ItemId, comp: &'a Comp, time: Tick) -> EvalCtx<'a> {
        EvalCtx { project, comp_id, comp, time, expr: None }
    }
    pub fn at(&self, time: Tick) -> EvalCtx<'a> {
        EvalCtx { time, ..*self }
    }
    /// Value of a property at the context time (keyframes, then expression).
    pub fn value(&self, layer: &Layer, prop: &Property) -> Value {
        // Separated Position reads as the combination of X/Y/Z Position.
        if prop.match_id == "position"
            && let Some(tr) = layer.transform()
            && let Some(px) = tr.get("positionX")
            && tr.get("position").is_some_and(|p| p.uid == prop.uid)
        {
            let z = tr.get("positionZ").map(|p| self.value(layer, p).as_f64()).unwrap_or(0.0);
            let y = tr.get("positionY").map(|p| self.value(layer, p).as_f64()).unwrap_or(0.0);
            return Value::Vec3([self.value(layer, px).as_f64(), y, z]);
        }
        let lt = layer.layer_time(self.time);
        let v = prop.value_at(lt);
        if prop.has_expression()
            && let Some(h) = self.expr
        {
            return h.eval(self, layer, prop, &v).unwrap_or(v);
        }
        v
    }
    pub fn group_value(&self, layer: &Layer, g: &PropGroup, m: &str) -> Option<Value> {
        g.get(m).map(|p| self.value(layer, p))
    }
    pub fn f(&self, layer: &Layer, g: &PropGroup, m: &str, d: f64) -> f64 {
        self.group_value(layer, g, m).map(|v| v.as_f64()).unwrap_or(d)
    }
    pub fn v2(&self, layer: &Layer, g: &PropGroup, m: &str, d: [f64; 2]) -> [f64; 2] {
        self.group_value(layer, g, m).map(|v| v.as_vec2()).unwrap_or(d)
    }
    pub fn v3(&self, layer: &Layer, g: &PropGroup, m: &str, d: [f64; 3]) -> [f64; 3] {
        self.group_value(layer, g, m).map(|v| v.as_vec3()).unwrap_or(d)
    }
    pub fn color(&self, layer: &Layer, g: &PropGroup, m: &str) -> [f32; 4] {
        self.group_value(layer, g, m).map(|v| v.as_color()).unwrap_or([1.0; 4])
    }
    pub fn e(&self, layer: &Layer, g: &PropGroup, m: &str) -> u32 {
        self.group_value(layer, g, m).map(|v| v.as_enum()).unwrap_or(0)
    }
    pub fn b(&self, layer: &Layer, g: &PropGroup, m: &str) -> bool {
        self.group_value(layer, g, m).map(|v| v.as_bool()).unwrap_or(false)
    }
    /// Layer Position, honouring Separate Dimensions (X/Y/Z Position properties).
    pub fn position(&self, layer: &Layer, tr: &PropGroup) -> [f64; 3] {
        match tr.get("positionX") {
            Some(px) => [self.value(layer, px).as_f64(), self.f(layer, tr, "positionY", 0.0), self.f(layer, tr, "positionZ", 0.0)],
            None => self.v3(layer, tr, "position", [0.0; 3]),
        }
    }
    /// Source time of a layer at the context time: Time Remap's value when enabled, else the
    /// (stretch-aware) layer time.
    pub fn source_time(&self, layer: &Layer) -> Tick {
        if let Some(tr) = layer.props.get("timeRemap") {
            return Tick::from_seconds_f64(self.value(layer, tr).as_f64());
        }
        layer.layer_time(self.time)
    }
    pub fn layer(&self, id: LayerId) -> Option<&'a Layer> {
        self.comp.layer(id)
    }

    /// Local (parent-space) transform of a layer.
    pub fn local_matrix(&self, layer: &Layer) -> Mat4 {
        let Some(tr) = layer.transform() else { return Mat4::IDENTITY };
        let three = layer.is_3d();
        let pos = self.position(layer, tr);
        let rz = self.f(layer, tr, "rotation", 0.0);
        if layer.is_camera() || layer.is_light() {
            return Mat4::translate(Vec3::from(pos)) * Mat4::rotate_z(rz);
        }
        let anchor = self.v3(layer, tr, "anchor", [0.0; 3]);
        let mut scale = self.v3(layer, tr, "scale", [100.0; 3]);
        if !three {
            scale[2] = 100.0;
            let a = Vec3::from([anchor[0], anchor[1], 0.0]);
            let p = Vec3::from([pos[0], pos[1], 0.0]);
            return Mat4::layer_3d(a, p, Vec3::from(scale), Vec3::ZERO, vec3(0.0, 0.0, rz));
        }
        let o = self.v3(layer, tr, "orientation", [0.0; 3]);
        let rx = self.f(layer, tr, "rotationX", 0.0);
        let ry = self.f(layer, tr, "rotationY", 0.0);
        Mat4::layer_3d(Vec3::from(anchor), Vec3::from(pos), Vec3::from(scale), Vec3::from(o), vec3(rx, ry, rz))
    }

    /// Layer space → comp (world) space, including parents.
    pub fn world_matrix(&self, layer: &Layer) -> Mat4 {
        let mut m = self.local_matrix(layer);
        let mut cur = layer.parent;
        let mut guard = 0;
        while let Some(pid) = cur {
            guard += 1;
            if guard > 64 {
                break;
            }
            let Some(p) = self.comp.layer(pid) else { break };
            m = self.local_matrix(p) * m;
            cur = p.parent;
        }
        m
    }

    pub fn opacity(&self, layer: &Layer) -> f64 {
        layer.transform().map(|tr| self.f(layer, tr, "opacity", 100.0)).unwrap_or(100.0) / 100.0
    }

    /// The comp's camera at this time: (view-projection to comp pixels, eye position).
    pub fn camera(&self) -> (Mat4, Vec3, f64) {
        let (w, h) = (self.comp.width as f64, self.comp.height as f64);
        if let Some(cam) = self.comp.active_camera(self.time)
            && let Some(tr) = cam.transform()
        {
            let pos = Vec3::from(self.v3(cam, tr, "position", [w / 2.0, h / 2.0, -1000.0]));
            let zoom = cam.props.sub("cameraOptions").map(|g| self.f(cam, g, "zoom", 1000.0)).unwrap_or(1000.0);
            let o = self.v3(cam, tr, "orientation", [0.0; 3]);
            let rx = self.f(cam, tr, "rotationX", 0.0);
            let ry = self.f(cam, tr, "rotationY", 0.0);
            let rz = self.f(cam, tr, "rotation", 0.0);
            // Extra camera rotation is applied in camera space (inverse, because it rotates the eye).
            let extra = (Mat4::orientation(Vec3::from(o)) * Mat4::rotate_z(rz) * Mat4::rotate_y(ry) * Mat4::rotate_x(rx)).transpose();
            let view = if let Some(poi) = tr.get("poi").map(|p| self.value(cam, p).as_vec3()) {
                look_at(pos, Vec3::from(poi), extra)
            } else {
                look_at(pos, pos + vec3(0.0, 0.0, 1.0), extra)
            };
            let mut view = view;
            // Parented cameras: apply the parent's world matrix inverse.
            if let Some(pid) = cam.parent
                && let Some(p) = self.comp.layer(pid)
                && let Some(inv) = self.world_matrix(p).inverse()
            {
                view = view * inv;
                let _ = pos;
            }
            return (effectcraft_geom::camera_matrix(w, h, pos, view, zoom), pos, zoom);
        }
        let zoom = effectcraft_geom::default_camera_zoom(w);
        let pos = vec3(w / 2.0, h / 2.0, -zoom);
        let view = look_at(pos, vec3(w / 2.0, h / 2.0, 0.0), Mat4::IDENTITY);
        (effectcraft_geom::camera_matrix(w, h, pos, view, zoom), pos, zoom)
    }

    /// Layer space → comp pixel space as a 2D projective matrix, plus camera-space depth of the
    /// layer's anchor (for 3D sorting). 2D layers ignore the camera.
    pub fn layer_to_comp(&self, layer: &Layer) -> (Mat3, f64) {
        let world = self.world_matrix(layer);
        if layer.is_3d() {
            let (cam, _, _) = self.camera();
            let full = cam * world;
            let anchor = layer.transform().map(|tr| self.v3(layer, tr, "anchor", [0.0; 3])).unwrap_or([0.0; 3]);
            let a = world.apply(Vec3::from(anchor));
            let depth = cam.0[3][0] * a.x + cam.0[3][1] * a.y + cam.0[3][2] * a.z + cam.0[3][3];
            (full.plane_to_mat3(), depth)
        } else {
            let m = &world.0;
            (Mat3([[m[0][0], m[0][1], m[0][3]], [m[1][0], m[1][1], m[1][3]], [0.0, 0.0, 1.0]]), 0.0)
        }
    }
}

/// Size of a layer's source in layer pixels.
pub fn source_size(project: &Project, layer: &Layer) -> (u32, u32) {
    match &layer.source {
        LayerSource::Footage { item } | LayerSource::Comp { item } | LayerSource::Solid { item } => {
            project.item(*item).and_then(|i| i.dimensions()).unwrap_or((0, 0))
        }
        LayerSource::Null => (100, 100),
        _ => (0, 0),
    }
}
