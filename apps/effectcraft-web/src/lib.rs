//! EffectCraft in the browser.
//!
//! The same engine and egui UI as the desktop app, compiled to `wasm32-unknown-unknown` and run
//! by eframe's web runner (WebGPU, WebGL2 fallback). What the desktop shell gets from the OS, this
//! crate gets from the browser:
//!
//! | Desktop | Web |
//! |---|---|
//! | file system (`FsServices`, media reads, export writes) | a virtual file table ([`files`]) persisted to the Origin Private File System / IndexedDB ([`persist`], [`store`]) |
//! | config directory (settings, shortcuts, recent projects, auto-saves) | the same store ([`store::WebConfig`]) |
//! | rfd file dialogs | `<input type=file>` pickers; drop files on the page |
//! | written files (Save, Render Queue) | browser downloads (several render files as one `.zip`) |
//! | frame render threads | `Frames::pump` on the UI thread between egui frames |
//! | Render Queue / analysis threads | Web Workers running their own engine instance ([`worker`]) |
//! | cpal audio output | Web Audio: an AudioWorklet fed from the preview mixdown ([`audio`]) |
//! | TCP control channel / MCP | `window.effectcraft` JavaScript API ([`api`]) |
//!
//! [`store`] is portable (unit-tested natively); everything else is `wasm32`-only.

pub mod store;

#[cfg(target_arch = "wasm32")]
pub mod api;
#[cfg(target_arch = "wasm32")]
mod app;
#[cfg(target_arch = "wasm32")]
pub mod audio;
#[cfg(target_arch = "wasm32")]
pub mod files;
#[cfg(target_arch = "wasm32")]
pub mod persist;
#[cfg(target_arch = "wasm32")]
pub mod worker;

#[cfg(target_arch = "wasm32")]
pub use app::*;
