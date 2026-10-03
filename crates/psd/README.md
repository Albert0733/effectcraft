# effectcraft-psd

Reads Photoshop documents (`.psd`, and `.psb` large documents) for File ▸ Import (as footage, as a
composition, or as a composition with layer sizes retained), and writes a minimal subset (used to
generate test fixtures).

Implemented from Adobe's public **Adobe Photoshop File Formats Specification** (November 2019
edition): file header, colour mode data, image resources, layer and mask information (layer
records, channel image data, layer mask data, additional layer information: `luni`, `lsct`/`lsdk`,
`lyid`, `iOpa`, `TySh`, `lfx2`, `SoCo`, `nvrt`, `brit`, `hue2`, `levl`, `expA`, `vmsk`/`vsms`,
`Lr16`/`Lr32`, smart objects: `SoLd`/`SoLE`/`PlLd` placement and `lnk2`/`lnkD`/`lnk3`/`lnkE`
linked layer data with embedded files), the descriptor (action) structure, and the merged image data. Raw, RLE (PackBits)
and ZIP (with and without prediction, inflated with `miniz_oxide`) compression; 1/8/16/32-bit;
Bitmap, Grayscale, Indexed, RGB, CMYK (naive conversion) and Lab colour modes.

Text engine data (`EngineData`) is read best-effort for the font, size, fill colour and
justification of the first style run. Smart objects import as footage of their embedded file
(a Photoshop document's merged image, or an image) placed by the smart object's corner quad
(position, scale, rotation; perspective and warps are not applied). Externally linked files
fall back to the layer's rendered pixels. No Adobe code, sample files or assets were used; test
fixtures are generated in-test by the writer in this crate ([`write`](src/write.rs)).
