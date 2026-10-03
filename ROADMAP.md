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
| M9 | Text and effects: text animators, layer styles, 259 effects incl. time and audio effects | Done (range, wiggly and expression selectors, per-character 3D, text on a path) |
| M10 | Export: render queue, H.264, ProRes, image sequences, GIF, audio | Done |
| M11 | Animation tools: audio playback, meters and waveforms, Lottie import and export (done); presets, Motion Sketch, Wiggler | In progress |
| M12 | Performance: layer cache and parallel compositing (done), GPU compositing, disk cache, motion tracking | In progress |
| M13+ | Puppet, paint, roto, motion tracking, a plugin API | Planned |
| M15 | The web app (WebAssembly, WebGPU) | Done, single-threaded |
| M14 | Built for agents: MCP server, command-line tool, control channel; Settings, keyboard shortcut editor, auto-save and crash recovery ([docs/preferences.md](docs/preferences.md)) | Done |

## How far from full parity

Measured feature by feature in [docs/parity.md](docs/parity.md) (end of 2 October 2026):

- **≈ 79% of After Effects' features, weighted by importance** (87% of the essentials, 67% of
  professional daily-use features, 21% of the long tail); 258 of 298 effects, 16 of them on the GPU.
- **≈ 100 agent-hours of work remain by the audit's estimate.** With five Claude Opus 5.5 agents in
  parallel that is **≈ 8–10 hours of wall-clock time at the pace measured so far, up to ≈ 25 hours**
  by the audit's conservative per-feature figures.
- Hardest remaining pieces: Roto Brush, Advanced 3D, a scripting object model (the 3D Camera Tracker landed).

Come tell us what matters most to you on [Discord](https://discord.gg/artcraft).
