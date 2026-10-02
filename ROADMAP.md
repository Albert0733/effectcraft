# Roadmap

EffectCraft aims to do what After Effects does, with the same panels, menus and behaviour, written
from scratch in Rust. This is where it stands. The milestones overlap; several are worked on at
once.

| | Milestone | State |
|---|---|---|
| M0 | Skeleton: crates, compositor, the After Effects style shell, control channel, `cargo xtask` | Done (web build pending) |
| M1 | Keyframes: temporal and spatial interpolation, Easy Ease, roving, velocity | Done |
| M2 | Compositing: 38 blend modes, track mattes, parenting, adjustment layers | Done |
| M3 | Project operations: settings dialogs, layer commands, `.ecproj`, undo, After Effects menu bar | Mostly done |
| M4 | Preview: precomps, motion blur, cached playback | Mostly done |
| M5 | Timeline depth: graph editor, keyframe clipboard and dialogs, time remapping, pick-whips, expression editor | Done |
| M6 | Shapes, masks and footage: shape operators, masks and the pen tool, video and image import | Done |
| M7 | 3D: 3D layers, cameras, lights, shadows, depth of field, 3D views and camera tools | In review |
| M8 | Expressions with the After Effects object model | Done |
| M9 | Text and effects: text animators, 241 effects | In progress (layer styles, more selectors) |
| M10 | Export: render queue, H.264, ProRes, image sequences, GIF, audio | Done |
| M11 | Animation tools: presets, Motion Sketch, Wiggler, Lottie, audio playback | Next |
| M12 | Performance: layer cache and parallel compositing (done), GPU compositing, disk cache, motion tracking | In progress |
| M13+ | Time effects, puppet, paint, roto, preferences and shortcut editor, the web app, a plugin API | Planned |
| M14 | Built for agents: MCP server, command-line tool, control channel | Done |

Come tell us what matters most to you on [Discord](https://discord.gg/artcraft).
