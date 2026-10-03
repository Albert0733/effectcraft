//! Layered and vector stills: Photoshop documents (`effectcraft-psd`: the merged image, or one
//! layer for footage imported as a composition) and SVG (`effectcraft-svg`, rasterised).

use effectcraft_project::{AlphaMode, Footage};
use effectcraft_raster::Image;

use crate::convert::{AlphaOp, straight_to_image};
use crate::{MediaError, Result};

fn is_svg(path: &str, bytes: &[u8]) -> bool {
    path.to_ascii_lowercase().ends_with(".svg") || effectcraft_svg::looks_like_svg(bytes)
}

/// Footage for a Photoshop or SVG file (`None` for other formats).
pub(crate) fn probe(path: &str, bytes: &[u8]) -> Result<Option<Footage>> {
    if effectcraft_psd::is_psd(bytes) {
        let psd = effectcraft_psd::Psd::parse(bytes.to_vec()).map_err(|e| MediaError::Decode(format!("{path}: {e}")))?;
        let mut f = crate::probe::still_footage(path, psd.width, psd.height, image::ImageFormat::Png, true);
        f.codec = if psd.psb { "PSB".into() } else { "PSD".into() };
        f.alpha = if psd.merged_alpha || !psd.layers.is_empty() { AlphaMode::Straight } else { AlphaMode::Ignore };
        return Ok(Some(f));
    }
    if is_svg(path, bytes) {
        let doc = effectcraft_svg::parse(bytes).map_err(|e| MediaError::Decode(format!("{path}: {e}")))?;
        let (w, h) = doc.pixel_size();
        let mut f = crate::probe::still_footage(path, w, h, image::ImageFormat::Png, true);
        f.codec = "SVG".into();
        f.alpha = AlphaMode::Straight;
        return Ok(Some(f));
    }
    Ok(None)
}

/// Decode a Photoshop or SVG still (`None` for other formats).
pub(crate) fn decode(path: &str, bytes: &[u8], footage: &Footage, op: AlphaOp) -> Result<Option<Image>> {
    if effectcraft_psd::is_psd(bytes) {
        let psd = effectcraft_psd::Psd::parse(bytes.to_vec()).map_err(|e| MediaError::Decode(format!("{path}: {e}")))?;
        let px = match &footage.layer {
            Some(l) => psd.layer_pixels(l.index as usize, !l.layer_size),
            None => psd.composite(),
        }
        .map_err(|e| MediaError::Decode(format!("{path}: {e}")))?;
        return Ok(Some(straight_to_image(px.width, px.height, &px.data, op)));
    }
    if is_svg(path, bytes) {
        let doc = effectcraft_svg::parse(bytes).map_err(|e| MediaError::Decode(format!("{path}: {e}")))?;
        let (w, h) = doc.pixel_size();
        let img = effectcraft_svg::rasterize(&doc, w, h, 1.0);
        // The rasteriser produces premultiplied pixels already.
        let _ = op;
        return Ok(Some(img));
    }
    Ok(None)
}

/// An SVG rasterised at `scale` × its pixel size (Continuously Rasterize).
pub(crate) fn rasterize_svg(bytes: &[u8], scale: f64) -> Option<Image> {
    let doc = effectcraft_svg::parse(bytes).ok()?;
    let (w, h) = doc.pixel_size();
    let (sw, sh) = (((w as f64 * scale).ceil() as u32).clamp(1, 16384), ((h as f64 * scale).ceil() as u32).clamp(1, 16384));
    Some(effectcraft_svg::rasterize(&doc, sw, sh, scale))
}
