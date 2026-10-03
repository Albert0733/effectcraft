//! GPU effects, noise, blur and time family (kernels in `shaders/fx_noise.wgsl`, every entry point prefixed
//! `fxn_`): the CPU effects' exact steps with the pixel loops as compute kernels.

use effectcraft_effects::EffectCtx;

use crate::context::Enc;
use crate::effects::GBuf;

/// Compute entry points in `fx_noise.wgsl`.
pub(crate) const KERNELS: &[&str] = &[];

/// Effect ids implemented here.
pub(crate) const IDS: &[&str] = &[];

/// Run effect `id` (one of [`IDS`]); `None` = this parameter combination runs on the CPU.
pub(crate) fn apply(e: &mut Enc, id: &str, ctx: &EffectCtx, b: GBuf) -> Option<GBuf> {
    let _ = (e, id, ctx, b);
    None
}
