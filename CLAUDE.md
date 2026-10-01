# EffectCraft — instructions for agents

EffectCraft is a clean-room, open-source, pure-Rust motion graphics and visual effects compositor targeting Adobe After Effects parity (and beyond). Native on macOS, Windows, Linux; web via WASM. Sibling of `../photocraft` (Photoshop), `../printcraft` (Acrobat), `../drawcraft` (Illustrator), `../filmcraft` (Premiere), `../lightcraft` (Lightroom) and `../designcraft` (InDesign), with the same conventions.

## Start every session here
`plan/` is maintainer-local (gitignored). Public equivalents: [`ROADMAP.md`](ROADMAP.md), [`docs/`](docs/).
1. Read `plan/STATUS.md` (current milestone, next task, running agents, blockers).
2. Read the task in `plan/execution-plan.md` §3, the relevant section of `plan/architecture.md`, and the crate README/docs you touch. Feature ids: `plan/aftereffects/feature-catalog.md`; effects: `plan/aftereffects/effects-list.md`; UI reference: `plan/aftereffects/README.md`.
3. Follow the autonomous operation protocol (`plan/execution-plan.md` §7). Don't stop to ask.

## Non-negotiables
**Read [`AGENTS.md`](AGENTS.md) first; its rules override everything here.** No Adobe assets, every asset licensed + attributed (`cargo xtask assets`).
- **Clean-room.** Behaviour and public docs only. No GPL/LGPL/AGPL code. ffmpeg only as an external test oracle.
- **Pure Rust.** Media codecs come from FilmCraft crates (git deps behind `crates/media`, see `plan/adr/0001`).
- **Layering** (`cargo xtask layers`): nothing below L5 depends on egui/eframe/winit/rfd/cpal; L0–L4 build for wasm32.
- **Exact time:** `effectcraft_time::Tick` (254 016 000 000/s). Keyframe times are **layer time**.
- **Property tree:** everything animatable is a `Property` in the layer's `PropGroup` tree, addressed by paths (`transform/position`, `effects/#1/blurriness`, `@uid`).
- **Everything is a command** (`crates/engine`): id, label, menu path, shortcut, params, enabled(), run(). UI, CLI, control channel and MCP dispatch by id.
- **Everything is agent-drivable:** every interactive widget registers an automation id; UI state is serde.
- **Community links** (Help menu, About, header Discord button): https://discord.gg/artcraft, https://getartcraft.com, https://getartcraft.com/apps/effectcraft, https://github.com/storytold/effectcraft.
- **Quality gates** before every commit: `cargo xtask ci` (fmt, clippy -D warnings, tests, layers, assets, wasm).
- **Commits:** one task id per commit (`M6.3: trim paths`). Only green states.

## Running and looking at the app
- `cargo run -p effectcraft -- --control 9877` opens the desktop app with the JSON-lines control server (`docs/control-protocol.md`).
- For UI work, **look at the result**: drive via the control channel and take `ui.screenshot`; or headless: `cargo run -p effectcraft-cli -- snapshot out.png`.
- Parallel agents: separate git worktrees and `CARGO_TARGET_DIR=target/agent-<name>`; keep every `Cargo.toml` valid (the `crates/*` glob).
