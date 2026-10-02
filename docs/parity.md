# Parity with After Effects

How close EffectCraft is to After Effects 2026, feature by feature, and how much work is left.
Audited against the code at commit `5273f1b` (2 October 2026). Every feature in our catalog was
graded from the code itself: a disabled menu entry, or a setting that is stored but never
rendered, does not count as done.

## Summary

| Measure | Value |
|---|---|
| **Feature parity, weighted by tier** (P0 ×3, P1 ×2, P2 ×1) | **≈ 64%** |
| Feature parity, unweighted | ≈ 58% |
| P0 (it isn't After Effects without it) | ≈ 75% |
| P1 (professional daily use) | ≈ 38% |
| P2 (long tail) | ≈ 17% |
| Features done / partial / missing | 18 / 60 / 14 of 92 |
| **Effects** | **257 of 298** After Effects 2026 effects by name (86%); ≈ 70% allowing for simplified implementations |
| Remaining work | ≈ **167 agent-hours** (139 h of features + ≈ 20% for 1:1 polish against After Effects) |
| Wall-clock estimate | ≈ **42–50 hours** with five agents working in parallel and one integrating; ≈ 31–35 h for P0 + P1 only |

"Agent-hours" are hours of one autonomous coding agent (Claude Opus 5.5) working in its own git
worktree with tests. The estimate is calibrated on this repository's own history: Classic 3D
(cameras, lights, shadows, depth of field, views, tools) took one agent about 2.7 h; timeline
depth (graph editor, keyframe dialogs, time remapping, pen and masks) about 2.3 h; 85 effects
about 1.2 h; the After Effects menu bar about 1.3 h; layer styles about 0.6 h; the web build
about 0.8 h. What is left is harder on average (tracking, roto, puppet, GPU, Advanced 3D), and
sequential chains (tracker → mask tracking → Warp Stabilizer → 3D camera tracker; GPU
compositor → GPU effects) limit how much can run in parallel.

Work in progress at the time of the audit, not yet counted: motion tracking, preferences and the
keyboard shortcut editor, auto-save and crash recovery, paint and puppet tools, Lottie import and
export.

## Update after the second wave (same day, commit `e1f7241`)

Motion tracking (1-, 2- and 4-point, stabilize, corner pin), paint (Brush, Clone Stamp, Eraser
with write-on) and the puppet tools, preferences, a keyboard shortcut editor that rebinds,
auto-save and crash recovery, Lottie import and export, Effect Controls widgets (angle dial,
point crosshair, eyedropper, curves and levels editors, effect copy/paste) and 20 more menu
commands landed. Re-scoring only the features they touch:

| Measure | Before | After |
|---|---|---|
| Feature parity, weighted | 63.7% | **≈ 70%** |
| P0 / P1 / P2 | 75% / 38% / 17% | 78% / 56% / 17% |
| Remaining, audit estimate | 167 agent-hours | ≈ 141 agent-hours |

**Calibration.** The audit priced this wave at about 21 agent-hours; five agents delivered it in
about 1.5 hours of wall-clock time (roughly 6 agent-hours of actual work), though some features
landed at 80–90% rather than fully done. So the per-feature estimates above are conservative.
Remaining wall-clock time with five parallel agents is therefore somewhere between **≈ 12–15
hours** at the pace observed so far and **≈ 35–40 hours** if the audit's figures hold. The
hardest remaining systems (Roto Brush, Warp Stabilizer, the 3D camera tracker, Advanced 3D, GPU
rendering, on-canvas text editing) are where the conservative figure is most likely to be right.

## By area

| Area | Weighted parity | Remaining (agent-hours) | Biggest gaps |
|---|---|---|---|
| Layers | 88% | 4.8 | frame blending, collapse transformations, slip edit |
| Output | 85% | 4.2 | multiple output modules, pre-render, WebM, audio-only |
| Audio | 85% | 0.5 | audio to keyframes |
| Import | 78% | 4.5 | PSD, SVG as shapes, Lottie |
| Automation | 76% | 3.3 | a JavaScript scripting object model |
| Shapes | 68% | 3.9 | pen tool for shape paths, vertex editing, taper and wave strokes |
| Compositions | 67% | 7.4 | marker dialog, flowchart, Essential Graphics |
| Animation | 65% | 10.4 | motion-path handles in the viewer, graph editor transform box, puppet, Wiggler/Smoother/Motion Sketch |
| Text | ≈ 85% | 1.5 | vertical type is basic (upright characters, no tate-chu-yoko), no OpenType feature panel, no text on 3D bevels; per-character styles, paragraph settings, on-canvas editing and the `sourceText` style API landed (M9.9–M9.10) |
| Web | 60% | 2.5 | threads, browser storage, audio |
| 3D | 58% | 20.5 | Advanced 3D (models, PBR, image-based light), 3D camera tracker, multi-view layouts |
| Effects | 56% | 14.6 | Effect Controls widgets (angle dial, point crosshair, eyedropper, curves), GPU effects, 41 missing effects |
| Interface | 55% | 10.4 | drag-to-dock and floating panels, viewer rulers/snapping/channels/snapshots, preferences, native macOS menus |
| Project | 53% | 8.1 | 8/16/32-bit pipeline and colour management, auto-save, proxies, folder moves |
| Masks & roto | 44% | 13.0 | mask tracking, Roto Brush, free transform and RotoBezier |
| Preview | 43% | 8.1 | GPU compositor, disk cache, region of interest, snapshots and exposure |
| Tracking | 0% | 13.0 | point tracker, Warp Stabilizer, mask and face tracking |
| Paint | 0% | 4.0 | Brush, Clone Stamp, Eraser |

## Effects still missing

Immersive Video (all 12), 3D Channel (all 8, including Cryptomatte), Color Stabilizer and the five
OCIO transforms, CC Flo Motion, Liquify, Rolling Shutter Repair, Warp Stabilizer, Compressor,
Distortion and Gate (audio), Camera-Shake Deblur, CC Radial Blur, CC Hair, Particle Playground,
Keylight (our Screen Key stands in), Color Profile Converter and the 3D Camera Tracker. Boris FX
Mocha and Cineware are third-party and not counted.

## Highest-value gaps, in order

1. ~~On-canvas text editing and per-character styles~~ (landed: M9.9–M9.10).
2. Effect Controls widgets: angle dial, point crosshair, eyedropper, curves and levels editors.
3. Viewer basics: snapping, rulers, channel view, snapshots, exposure, a drawable region of interest.
4. A GPU (wgpu) compositor, then GPU effects.
5. Pen tool for shape paths, shape vertex editing, the Layer viewer, free transform and RotoBezier.
6. Motion-path handles in the viewer; graph editor transform box and snapping.
7. Point tracking and stabilization (in progress).
8. Frame blending, collapse transformations, slip edit.
9. A real 8/16/32-bit pipeline with linear blending and colour management.
10. Drag-to-dock and floating panels, saved workspaces, native macOS menus.
11. Auto-save, crash recovery, recent projects (in progress).
12. Expression gaps: `sampleImage`, `footage()` (the `sourceText` style API landed in M9.9).
13. Lottie (in progress), WebM, SVG and PSD import.
14. Puppet and paint tools (in progress).
15. Preferences and a shortcut editor that can rebind (in progress); real Wiggler, Smoother and Motion Sketch; the marker dialog.

The engine underneath (keyframes, expressions, shape operators, text animators, Classic 3D, all
38 blend modes, the render queue) is deep. Most of what is missing is interactive tooling in the
viewer, settings that are stored but not yet rendered, about 69 menu entries that are still
disabled, and the large systems: tracking, puppet, paint, roto, Advanced 3D and GPU rendering.
