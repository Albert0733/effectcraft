# effectcraft-pdf

Vector footage from PDF files, PDF-compatible Adobe Illustrator files (`.ai`, which are PDF
documents) and Encapsulated PostScript (`.eps`). Files parse into the same render tree as SVG
(`effectcraft_svg::Doc`), so they rasterise at any scale (Continuously Rasterize), convert to shape
layers (Layer ▸ Create ▸ Create Shapes from Vector Layer) and import as compositions with one
layer per file layer (File ▸ Import ▸ Composition).

Specifications used: **ISO 32000-1:2008** (PDF 1.7; the public Adobe edition) for the file
structure, filters, content streams, graphics state, colour spaces, functions, shadings, text,
fonts, images, patterns, transparency (blend modes, soft masks) and optional content; Adobe
**Technical Note #5176** (The Compact Font Format Specification) and **#5177** (The Type 2
Charstring Format) for CFF font programs; **Adobe Type 1 Font Format** (version 1.1) for Type 1
font programs; the **PostScript Language Reference, third edition** and the **Encapsulated
PostScript File Format Specification 3.0** for EPS. TrueType font programs are read through
skrifa and JPEG (DCT) images through zune-jpeg (both permissively licensed, pure Rust). No
third-party PDF/PostScript/font code was read; test files (including the CFF and Type 1 fonts)
are generated in code ([`write`](src/write.rs) and the tests).

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

- **Text** (§9): text objects and state (`Tc`, `Tw`, `Tz`, `TL`, `Ts`, `Tr`, `Tf`, `Td`,
  `TD`, `Tm`, `T*`), `Tj`, `TJ` (with kerning), `'` and `"`; render modes fill / stroke /
  fill-and-stroke / invisible and the clipping modes (glyph outlines clip until `Q`). Glyphs
  become outlines (one compound shape per show operator, named "Text: …" after the string
  through `ToUnicode` or the glyph names). Fonts: simple fonts with `/Widths`, base encodings
  (Standard, WinAnsi, MacRoman), `/Differences` and built-in encodings; composite (Type 0) fonts
  with Identity-H / Identity-V and embedded CMaps (`cidrange` / `cidchar`), `/W` and `/DW`
  widths and `CIDToGIDMap`. Embedded programs: TrueType (`FontFile2`, cmap (3,1) / (3,0) /
  (1,0) and `post` names), CFF (`FontFile3` `/Type1C`, `/CIDFontType0C`, `/OpenType`;
  name-keyed and CID-keyed with FDArray / FDSelect, flex, hint masks, `seac`), Type 1
  (`FontFile`, eexec, `/Subrs`, flex and hint replacement through othersubrs, `seac`) and Type 3
  glyph procedures. Fonts that are not embedded (the standard 14 and others) are drawn with
  the bundled OFL fonts by name: Inter (sans serif, Regular / Italic / SemiBold / Bold), Noto
  Serif (Times and other serif names) and JetBrains Mono (Courier), with their own advances
  when the PDF gives no widths.
- **Images** (§8.9): image XObjects and inline images (`BI … ID … EI`, abbreviated keys);
  1, 2, 4, 8 and 16 bits per component with `/Decode`; any colour space above (Indexed through
  its palette, ICCBased through its `/Alternate` space); DCT (JPEG, through zune-jpeg; Adobe
  inverted CMYK) and the general filters; stencil masks (`/ImageMask`) in the fill colour;
  soft masks (`/SMask` images) and colour-key and explicit `/Mask`s. Images become image nodes
  of the render tree (bilinear, box-filtered when minified).
- **Transparency:** blend modes (`/BM`, all 16 PDF modes) and soft masks in the graphics state
  (`/SMask` with `/S /Luminosity` and `/BC` backdrops, or `/S /Alpha`) as groups of the render
  tree, open until the state changes or `Q`.
- **Patterns:** tiling patterns (coloured and uncoloured, `XStep` / `YStep`, the cell clipped
  to `/BBox`, at most 4096 tiles per fill) for fills; strokes with a tiling pattern use its
  colour (uncoloured) or grey.
- **Functions:** sampled, exponential, stitching and PostScript calculator (type 4) functions.
- **Pages:** any page ([`parse_page`], [`page_count`]); File ▸ Import takes `page` (from 1) and
  the Import dialog asks for it; footage remembers its page (`Footage::page`).

## Limitations

- Mesh shadings (types 4–7) and function-based shadings (type 1), JPX / CCITT / JBIG2 images,
  predefined CJK CMaps other than Identity, vertical metrics (`/W2`; vertical text advances by
  the font size) and transfer functions (`/TR`) are not read; knockout and isolated transparency
  groups composite as normal groups, and a soft mask applies to each painting operation while
  it is set rather than to their composite. What a document skipped is listed in `Doc::skipped`.
- Illustrator EPS files whose drawing depends on Adobe's procedure-set resources (defined in
  the file through constructs beyond this subset) and Level 3 `shfill` gradients in EPS draw
  only what the subset understands; save such artwork as PDF-compatible `.ai` or PDF instead.
  EPS text is not drawn.
- **Create Shapes from Vector Layer:** clips that enclose the whole page become layer masks
  (the first clip Add, later ones Intersect). Nested clipping groups become Merge Paths: each
  closed shape inside them is a group with its geometry merged into one compound path, the clip
  paths, and Merge Paths ▸ Intersect before its fill and stroke, so fills match exactly and
  strokes follow the clipped outline; open stroked paths inside clips stay unclipped. Blend
  modes become the groups' Blend Mode; images and soft masks are not converted (shape layers
  have no images; use the footage, or Import As: Composition).
