# effectcraft-pdf

Vector footage from PDF files, PDF-compatible Adobe Illustrator files (`.ai`, which are PDF
documents) and Encapsulated PostScript (`.eps`). Files parse into the same render tree as SVG
(`effectcraft_svg::Doc`), so they rasterise at any scale (Continuously Rasterize), convert to shape
layers (Layer ▸ Create ▸ Create Shapes from Vector Layer) and import as compositions with one
layer per file layer (File ▸ Import ▸ Composition).

Specifications used: **ISO 32000-1:2008** (PDF 1.7; the public Adobe edition) for the file
structure, filters, content streams, graphics state, colour spaces, functions, shadings and
optional content; the **PostScript Language Reference, third edition** and the **Encapsulated
PostScript File Format Specification 3.0** for EPS. No third-party PDF/PostScript code was read;
test files are generated in code ([`write`](src/write.rs) and the tests).

## What is read

- **Structure:** indirect objects found by scanning (damaged or incremental cross-reference
  sections do not matter), object streams, the page tree with inherited resources, MediaBox /
  CropBox and `/Rotate`. Filters: Flate (with PNG predictors), LZW, ASCIIHex, ASCII85,
  RunLength. Encrypted files are refused.
- **Content:** `q`/`Q`, `cm`, line width / cap / join / miter / dash, ExtGState (`CA`, `ca`, `LW`,
  `LC`, `LJ`, `ML`, `D`), all path construction and painting operators (non-zero and even-odd),
  clipping (`W`, `W*`) as clipping groups, colour in DeviceGray / RGB / CMYK, ICCBased (by
  component count), CalGray / CalRGB, Lab, Indexed, Separation and DeviceN (tint transforms of
  function types 0, 2 and 3), axial and radial shadings as pattern fills and through `sh`, form
  XObjects (matrix, bounding-box clip, nested resources).
- **Layers:** top-level optional-content marked sequences (`/OC … BDC`, Illustrator's layers)
  become the document's layers, named after their OCG; content outside them is gathered into
  layers "Layer N".
- **EPS:** a PostScript interpreter for the subset exported vector EPS uses: operand and
  dictionary stacks, procedures and `def`/`bind`/`load`/`where`, arithmetic, comparisons,
  `if`/`ifelse`/`for`/`repeat`/`loop`/`forall`, arrays and strings, `gsave`/`grestore`,
  `save`/`restore`, matrices (`concat`, `translate`, `scale`, `rotate`, `transform` …), path
  construction including `arc`/`arcn` and relative operators, `fill`/`eofill`/`stroke`,
  `clip`/`eoclip`, `rectfill`/`rectstroke`/`rectclip`, gray / RGB / CMYK / HSB colour. DOS EPS
  binary headers and `%%HiResBoundingBox` / `%%BoundingBox` are honoured; `%%BeginData` /
  `%%BeginBinary` / preview sections are skipped.

## Limitations

- Text is not drawn (embedded fonts are not read); images, tiling patterns, mesh shadings
  (types 4–7), PostScript calculator functions, soft masks and blend modes are skipped. What a
  document skipped is listed in `Doc::skipped`.
- Illustrator EPS files whose drawing depends on Adobe's procedure-set resources (defined in
  the file through constructs beyond this subset) and Level 3 `shfill` gradients in EPS draw
  only what the subset understands; save such artwork as PDF-compatible `.ai` or PDF instead.
- Shape-layer conversion has no clipping: clipped artwork converts unclipped.
- Only the first page of a multi-page PDF is imported.
