# Roadmap

EffectCraft aims to do what After Effects does, with the same panels, menus and behaviour, written
from scratch in Rust. This is where it stands. The milestones overlap; several are worked on at
once.

| | Milestone | State |
|---|---|---|
| M0 | Skeleton: crates, compositor, the After Effects style shell, control channel, `cargo xtask` | Done (web build: [docs/web.md](docs/web.md)) |
| M1 | Keyframes: temporal and spatial interpolation, Easy Ease, roving, velocity | Done |
| M2 | Compositing: 38 blend modes, track mattes, parenting, adjustment layers | Done |
| M3 | Project operations: settings dialogs, layer commands, `.ecproj`, undo, After Effects menu bar | Mostly done |
| M4 | Preview: precomps, motion blur, cached playback | Mostly done |
| M5 | Timeline depth: graph editor, keyframe clipboard and dialogs, time remapping, pick-whips, expression editor | Done |
| M6 | Shapes, masks and footage: shape operators, masks and the pen tool, video and image import | Done |
| M7 | 3D: 3D layers, cameras, lights, shadows, depth of field, 3D views and camera tools | In review |
| M8 | Expressions with the After Effects object model | Done |
| M9 | Text and effects: text animators, layer styles, 306 effects (all 298 of After Effects') incl. time and audio effects | Done (range, wiggly and expression selectors, per-character 3D, text on a path) |
| M10 | Export: render queue, H.264, ProRes, image sequences, GIF, audio | Done |
| M11 | Animation tools: audio playback, meters and waveforms, Lottie import and export, presets, Motion Sketch, Wiggler, Smoother | Done |
| M12 | Performance: layer cache, parallel and GPU compositing, disk cache, motion tracking (done); GPU versions of the remaining CPU-only effects | Mostly done |
| M13 | Puppet tools, paint, Roto Brush, motion tracking, a plug-in API, Timeline depth | Done (Roto Brush and face tracking use classical models; learned models are still to come) |
| M15 | The web app (WebAssembly, WebGPU) | Done: browser storage, Web Audio, renders and analyses in Web Workers, viewer frames and GPU effects in frame workers with their own WebGPU devices, the disk cache in the Origin Private File System, a storage manager, offline install ([docs/web.md](docs/web.md)) |
| M14 | Built for agents: MCP server, command-line tool, control channel; Settings, keyboard shortcut editor, auto-save and crash recovery ([docs/preferences.md](docs/preferences.md)) | Done |

## How far from full parity

Measured feature by feature in [docs/parity.md](docs/parity.md) (4 October 2026):

- **≈ 99% of After Effects' features, weighted by importance**, counting partial features as half
  done (≈ 99.8% with per-feature fractions): 89 of 92 features done, 3 partial, none missing; every
  essential (P0) feature done; all 306 effects implemented in full, 280 of them on the GPU.
- **≈ 8.5 agent-hours of work remain**: about **2–3 hours of wall-clock time** with five Claude
  Opus 5.5 agents in parallel, ≈ 1 hour without learned models for Roto Brush and face tracking.
- What is left: GPU kernels for the remaining visual effects, and learned-model quality for Roto
  Brush and face tracking.

Come tell us what matters most to you on [Discord](https://discord.gg/artcraft).
