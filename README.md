<p align="center">
  <a href="https://getartcraft.com/">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="docs/brand/artcraft-logo-white.svg">
      <img alt="ArtCraft" src="docs/brand/artcraft-logo.svg" width="200">
    </picture>
  </a>
</p>

<h1 align="center">EffectCraft</h1>

<p align="center">
  <b>Motion graphics and visual effects, in pure Rust.</b>
</p>

<p align="center">
  A free, open-source compositor in the spirit of After Effects: compositions, layers,
  keyframes, effects and expressions, native on macOS, Windows and Linux, and in the browser later on.
  It is in early development. The engine crates are being written now and there is no app to run yet.
</p>

<p align="center">
  <img alt="Status: early development" src="https://img.shields.io/badge/status-early%20development-e0368f?style=flat-square">
  <img alt="Written in Rust" src="https://img.shields.io/badge/rust-1.95%2B-b0206c?style=flat-square&logo=rust&logoColor=white">
  <img alt="License: MIT or Apache-2.0" src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-555?style=flat-square">
</p>

<p align="center">
  <a href="https://discord.gg/artcraft"><img alt="Join the ArtCraft community on Discord" src="https://img.shields.io/badge/Join%20us%20on%20Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" height="40"></a>
</p>

<p align="center">
  <a href="https://getartcraft.com/apps/effectcraft"><b>EffectCraft on getartcraft.com</b></a> ·
  <a href="https://getartcraft.com/">ArtCraft</a> ·
  <a href="https://getartcraft.com/apps">All Crafting Apps</a>
</p>

> [!NOTE]
> **ArtCraft is a community of artists from all walks of life.** Painters, photographers,
> filmmakers, illustrators, designers, animators, hobbyists, and people who picked up a pencil
> last week. If you make things, you're one of us. **[Come say hi on Discord](https://discord.gg/artcraft).**

<p align="center">
  <a href="#what-effectcraft-is">What it is</a> ·
  <a href="#where-it-stands">Where it stands</a> ·
  <a href="#the-plan">The plan</a> ·
  <a href="#building-from-source">Building</a> ·
  <a href="#how-its-made">How it's made</a> ·
  <a href="#the-crafting-apps">The Crafting Apps</a> ·
  <a href="#license-and-credits">License</a>
</p>

## What EffectCraft is

EffectCraft is for animated titles, motion graphics and compositing work: the kind of thing
people reach for After Effects to do. You build a composition out of layers (solids, shapes,
text, footage, other compositions), animate their properties with keyframes, stack effects on
them and render the result.

The aim is to feel familiar to anyone who has used After Effects, with the same panels (Project,
Composition, Timeline, Effect Controls, Effects & Presets) and the same keyframe behaviour, and
then to go further in a few places where it matters to us:

- **Lottie import and export built in**, so animations can go straight to the web and apps
  without a plugin.
- **A project file you can read.** Projects are versioned JSON (`.ecproj`), so they diff cleanly
  in version control.
- **Everything is scriptable.** Every menu item, timeline drag and property edit goes through one
  command registry, so the same actions are reachable from a command line, a JSON control
  channel and an MCP server for agents.
- **No FFmpeg.** Video and audio decoding and encoding come from FilmCraft's own pure-Rust
  codecs.

## Where it stands

> [!IMPORTANT]
> **There is no app to run yet.** EffectCraft is in its first milestone. The planning is done and
> the low-level engine crates are being written. The windowed app, the renderer and the effects
> come next. If you want to follow along or help shape it, the
> [Discord](https://discord.gg/artcraft) is the place.

| | What | State |
|---|---|---|
| **Built** | The plan: an After Effects UI and feature reference, a full effects list, the architecture and a milestone plan (see [`plan/`](plan/)) | Done |
| **In progress** | Engine foundation crates: time, geometry, color and blend modes, keyframes, the project model, raster images, Bezier paths, text | Early code and first tests |
| **Next** | The compositor, a first set of effects, the editing engine and the After Effects style interface rendering an animated demo composition | Not started |
| **Later** | Precomps and preview, shapes and masks, 3D, expressions, text animators, export, GPU rendering, the web build | Planned |

The live checklist is [`plan/STATUS.md`](plan/STATUS.md).

### The crates so far

| Crate | What it does |
|---|---|
| `effectcraft-time` | Exact media time in integer ticks, so 23.976 and 29.97 fps, drop-frame timecode and audio sample rates never drift |
| `effectcraft-geom` | 2D and 3D vectors, matrices and the layer transform (anchor, position, scale, rotation, orientation) |
| `effectcraft-color` | RGBA color, sRGB transfer, HSL and HSV, and the 38 After Effects blend modes |
| `effectcraft-keyframe` | Animated values: linear, hold and Bezier keyframes with speed and influence, and spatial motion paths |
| `effectcraft-project` | The document: project items, compositions, layers and the property tree |
| `effectcraft-raster` | Floating-point premultiplied images with warps, blurs and compositing, parallel across cores |
| `effectcraft-path` | Bezier shapes, path operations (trim, round corners, zig zag, offset and more), stroking and anti-aliased fills |
| `effectcraft-text` | Fonts, shaping, bidirectional text, line breaking and glyph outlines for text layers |

These are moving fast. Expect names and APIs to change.

## The plan

EffectCraft is built in milestones. The first one is about getting something you can see on
screen as early as possible, then filling in depth underneath.

| | Milestone | What you'll get |
|---|---|---|
| **M0** | Skeleton and visual shell | The full panel layout over a real CPU compositor playing an animated demo composition, about 8 effects, and the control channel and MCP server |
| M1 | Keyframes | Complete temporal and spatial interpolation, Easy Ease, roving keys, velocity |
| M2 | Compositing | All 38 blend modes, track mattes, parenting, 8/16/32 bit per channel behaviour |
| M3 | Project operations | Composition and layer settings, layer commands, save and open `.ecproj`, undo, clipboard |
| M4 | Preview and precomps | Nested compositions, adjustment layers, motion blur, cached RAM preview with audio |
| M5 | Timeline depth | Keyframe editing, the graph editor, markers, trims, time remapping |
| M6 | Shapes, masks and footage | Shape layers and path operations, masks, the pen tool, video and image import |
| M7 | 3D | 3D layers, cameras and lights |
| M8 | Expressions | JavaScript expressions with the After Effects object model (`wiggle`, `loopOut`, `thisComp` and friends) |
| M9 | Text and effects | Text animators, text on a path, layer styles, the first 60 effects |
| M10 | Export | Render queue, image sequences, H.264 and ProRes, GIF, command-line rendering |
| M11 | Animation tools | Motion Sketch, Wiggler, presets, Lottie import and export, audio |
| M12 | Performance | GPU compositing and effects, disk cache, motion tracking |
| M13 to M16 | Beyond | More effects, puppet and paint tools, polish, the web app, a plugin API |

The details, with an acceptance test for every task, are in
[`plan/execution-plan.md`](plan/execution-plan.md) and [`plan/architecture.md`](plan/architecture.md).

## Building from source

You need [Rust](https://rustup.rs/) 1.95 or newer. There is nothing to launch yet, but you can
build and test the engine crates:

```sh
git clone https://github.com/storytold/effectcraft
cd effectcraft
cargo build
cargo test
```

To work on one crate:

```sh
cargo test -p effectcraft-keyframe
```

The workspace picks up every crate under `crates/`, so while a new crate is half-written the
whole build can break for a while. The `cargo xtask` helpers named in the plan (layer checks,
asset checks, CI) are not written yet.

## How it's made

- **Engine first.** A stack of small crates with strict layering. The interface (egui) sits on
  top and nothing underneath depends on it.
- **Clean room.** We work from After Effects' public documentation and how it behaves, never
  from Adobe's files, icons, presets or code. No code is copied from GPL projects. The full rules
  are in [`plan/README.md`](plan/README.md).
- **Private by default.** No telemetry, and no network access unless you ask for it.
- **One family.** EffectCraft shares its time model and text engine design with FilmCraft, and
  gets its video and audio codecs from it (see [`plan/adr/`](plan/adr/)).

## The Crafting Apps

EffectCraft is one of the **Crafting Apps**: free, open-source creative tools from the
[ArtCraft](https://getartcraft.com/) team, each written from scratch in Rust and each able to
stand on its own.

| App | What it's for | Code | Learn more |
|---|---|---|---|
| <img src="https://img.shields.io/badge/PhotoCraft-2f7bf5?style=for-the-badge" alt="PhotoCraft" height="24"> | Image editing: layers, masks, type and real PSD files | [GitHub](https://github.com/storytold/photocraft) | [getartcraft.com](https://getartcraft.com/apps/photocraft) |
| <img src="https://img.shields.io/badge/VectorCraft-e8573f?style=for-the-badge" alt="VectorCraft" height="24"> | Vector illustration (formerly DrawCraft) | [GitHub](https://github.com/storytold/vectorcraft) | [getartcraft.com](https://getartcraft.com/apps/drawcraft) |
| <img src="https://img.shields.io/badge/FilmCraft-8b5cf6?style=for-the-badge" alt="FilmCraft" height="24"> | Video editing, color and sound | [GitHub](https://github.com/storytold/filmcraft) | [getartcraft.com](https://getartcraft.com/apps/filmcraft) |
| <img src="https://img.shields.io/badge/LightCraft-f2a516?style=for-the-badge" alt="LightCraft" height="24"> | Photo library and raw development | [GitHub](https://github.com/storytold/lightcraft) | [getartcraft.com](https://getartcraft.com/apps/lightcraft) |
| <img src="https://img.shields.io/badge/PrintCraft-12a58a?style=for-the-badge" alt="PrintCraft" height="24"> | Reading, organizing and protecting PDFs | [GitHub](https://github.com/storytold/printcraft) | [getartcraft.com](https://getartcraft.com/apps/printcraft) |
| <img src="https://img.shields.io/badge/EffectCraft-e0368f?style=for-the-badge" alt="EffectCraft" height="24"> | **Motion graphics and visual effects** · **you are here** | [GitHub](https://github.com/storytold/effectcraft) | [getartcraft.com](https://getartcraft.com/apps/effectcraft) |
| <img src="https://img.shields.io/badge/DesignCraft-7bb51c?style=for-the-badge" alt="DesignCraft" height="24"> | Page layout and publishing | [GitHub](https://github.com/storytold/designcraft) | [getartcraft.com](https://getartcraft.com/apps/designcraft) |

And [**ArtCraft**](https://getartcraft.com/) itself, our AI image and video studio for artists who want real control.

<br>

<p align="center">
  <a href="https://discord.gg/artcraft"><img alt="Join the ArtCraft community on Discord" src="https://img.shields.io/badge/Join%20us%20on%20Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" height="40"></a>
</p>

<h3 align="center">Come make things with us</h3>

<p align="center">
  Our Discord is where artists of every kind hang out: people who paint, shoot, draw, cut film,
  set type, and people still figuring out what they like to make. Share what you're working on,
  ask for help, tell us what's broken, or tell us what you wish these tools could do.
  Whatever your medium and however long you've been at it, you're welcome here.
</p>

<p align="center">
  <a href="https://discord.gg/artcraft"><b>discord.gg/artcraft</b></a> ·
  <a href="https://getartcraft.com/">getartcraft.com</a> ·
  <a href="https://getartcraft.com/apps">The Crafting Apps</a> ·
  <a href="https://getartcraft.com/apps/effectcraft">EffectCraft</a>
</p>

## License and credits

EffectCraft is dual-licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your
option. Bundled fonts and other non-code assets are listed with their authors and licenses in
[ATTRIBUTION.md](ATTRIBUTION.md).

After Effects is a trademark of Adobe. EffectCraft is an independent project and is not
affiliated with or endorsed by Adobe.

<p align="center">
  <a href="https://getartcraft.com/"><img alt="ArtCraft" src="docs/brand/artcraft-mark.svg" width="28"></a><br>
  <sub>Made by the <a href="https://getartcraft.com/">ArtCraft</a> team and community.</sub>
</p>
