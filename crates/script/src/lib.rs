//! # effectcraft-script (L4)
//!
//! Scripting with an After Effects-style object model, on the boa JavaScript engine (the one the
//! expressions use). Scripts written for After Effects' documented scripting API mostly run
//! unchanged:
//!
//! ```text
//! app.beginUndoGroup("Build");
//! var comp = app.project.items.addComp("Main", 1920, 1080, 1, 10, 30);
//! var solid = comp.layers.addSolid([1, 0, 0], "Red", 1920, 1080, 1);
//! solid.property("ADBE Transform Group").property("ADBE Position").setValueAtTime(0, [0, 540]);
//! solid.Effects.addProperty("ADBE Gaussian Blur 2").property("Blurriness").setValue(20);
//! app.endUndoGroup();
//! ```
//!
//! * **Object model** (`prelude.js`): `app` (project, open/newProject, undo groups,
//!   `executeCommand`/`findMenuCommandId`, `scheduleTask`), `Project`, `ItemCollection`,
//!   `CompItem`/`FootageItem`/`FolderItem`, `LayerCollection`, `AVLayer`/`TextLayer`/
//!   `ShapeLayer`/`CameraLayer`/`LightLayer`, `PropertyGroup`/`Property` (values, keyframes,
//!   eases, expressions), `MaskPropertyGroup`, `TextDocument`, `Shape`, `KeyframeEase`,
//!   `MarkerValue`, `RenderQueue`/`RenderQueueItem`/`OutputModule`, `ImportOptions`, `File`/
//!   `Folder`, `$`, `alert`/`writeLn`, and the enums (`BlendingMode`, `KeyframeInterpolationType`,
//!   `TrackMatteType`, `LightType`, `ParagraphJustification`, `PropertyValueType`…).
//! * **Edits are engine commands**: every mutating call runs a command through
//!   [`Session::execute`](effectcraft_engine::Session::execute), so it is undoable, journaled
//!   and identical to the UI's action; `app.beginUndoGroup`/`endUndoGroup` fold everything in
//!   between into one undo step.
//! * **Match names** ([`matchnames`]): After Effects' documented match names for our properties
//!   and effects (`ADBE Transform Group`, `ADBE Position`, `ADBE Gaussian Blur 2`…).
//! * **Security**: scripts can import/open/save projects and render, but `File` reads are limited
//!   to the project's folder and writes (and the network) are off unless Preferences ▸ Scripting
//!   & Expressions ▸ Allow Scripts to Write Files and Access Network is on.
//!
//! Entry points: [`install`] sets [`Session::script`](effectcraft_engine::Session::script), which
//! the `script.run` command, File ▸ Scripts ▸ Run Script File… (`.jsx`/`.js`), the Script Console
//! panel, `effectcraft-cli script` and the MCP `run_script` tool use.

#![recursion_limit = "256"]

pub mod matchnames;
mod model;
mod runtime;

use effectcraft_engine::{ScriptRequest, Session};

pub use runtime::{Outcome, ScriptError, run};

/// The [`effectcraft_engine::ScriptRunner`] this crate provides.
pub fn runner(s: &mut Session, req: &ScriptRequest) -> serde_json::Value {
    run(s, req).to_json()
}

/// Enable scripting on a session.
pub fn install(s: &mut Session) {
    s.script = Some(runner);
}

/// Run `code` as a one-off script named `name`.
pub fn run_code(s: &mut Session, code: &str, name: &str) -> Outcome {
    run(s, &ScriptRequest { code, name, console: false })
}

#[cfg(test)]
mod tests;
