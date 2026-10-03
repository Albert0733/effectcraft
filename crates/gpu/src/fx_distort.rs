//! GPU effects, distortion and stylize family (kernels in `shaders/fx_distort.wgsl`): the CPU effects' exact
//! steps with the pixel loops as compute kernels.

use effectcraft_effects::EffectCtx;

use crate::context::Enc;
use crate::effects::GBuf;

/// Compute entry points in `fx_distort.wgsl`.
pub(crate) const KERNELS: &[&str] = &[];

/// Effect ids implemented here.
pub(crate) const IDS: &[&str] = &[];

/// Run effect `id` (one of [`IDS`]); `None` = this parameter combination runs on the CPU.
pub(crate) fn apply(_e: &mut Enc, _id: &str, _ctx: &EffectCtx, _b: GBuf) -> Option<GBuf> {
    None
}
