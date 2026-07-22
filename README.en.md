<p align="right">
  <a href="./README.md">简体中文</a> · <strong>English</strong>
</p>

# OpenDeskTools

A local-first desktop productivity toolbox for Windows. It brings frequent actions into the current workflow through a consistent set of global hotkeys, clipboard history, and a quick-launch surface.

> OpenDeskTools is still in early development and does not provide an official installer yet. The current version is intended for development, testing, and product experience validation.

## What we are building

Clipboard history, application launching, region capture, image pinning, and QR conversion are often spread across several Windows utilities. OpenDeskTools aims to combine them into one lightweight, composable, keyboard-first toolbox:

- **Available on demand**: open a temporary surface with a global hotkey without leaving the active window or losing the current input context.
- **Local-first**: clipboard history, preferences, and icon caches stay on the device; core workflows do not depend on a cloud service.
- **One consistent experience**: the main window manages features while independent surfaces handle quick actions, both backed by the same data and domain rules.
- **Native Windows capabilities**: Rust, Tauri, and Windows APIs handle hotkeys, clipboard access, windows, the tray, and the future capture pipeline.
- **Built to remain maintainable**: shared services, components, and design tokens keep behavior and styling from diverging between entry points.

## Current status

| Area | Status | Available today |
| --- | --- | --- |
| Clipboard | Implemented, validation ongoing | Text, image, and file history; search and filters; favorites, deletion, editing, source icons; quick surface; copy and paste-to-target flows |
| Quick launch | Implemented, refinement ongoing | Application discovery and manual add; pinning, ordering, visibility, and removal; wheel/horizontal/vertical surfaces; real application icons |
| Global hotkeys | Implemented, compatibility testing ongoing | Visual rebinding, native key capture, conflict states, Windows shortcut handling, and runtime routing |
| QR conversion | Implemented | Generate a QR image from the latest internal text or decode one from an image, then save the result to internal history and attempt to sync the system clipboard |
| Theme and general settings | Core flow implemented | Light/dark themes, accent color, motion/transparency preferences, tray behavior, autostart, data-directory migration, and local diagnostics preference |
| F1 region capture | Planned | In-house multi-monitor capture pipeline, region selection, clipboard output, and file output |
| F3 image pinning | Planned | Borderless always-on-top image windows with drag, resize, and multiple instances |

See the [development plan](./docs/development-plan.md) for detailed acceptance gates, remaining real-device validation, and the planned implementation order.

## Product structure

OpenDeskTools has two complementary interaction layers:

1. **Main window**: manages hotkeys, quick launch, clipboard, QR conversion, themes, and general preferences.
2. **Quick surfaces**: open clipboard history or the tool menu near the current pointer/input context, then dismiss with a short transition without pulling the user away from the active task.

Backend capabilities are owned by a single service or manager. Tauri commands, global-hotkey callbacks, tray actions, and React pages remain thin entry adapters. See the [capability map](./docs/architecture/capability-map.md) for the full boundary model.

## Technology

- Rust + Tauri 2
- React + TypeScript + Vite
- Windows API / Win32
- Local SQLite persistence
- Vitest + Rust tests + GitHub Actions

Windows 10 and 11 are the primary development and validation environments today. A dedicated Windows 7 compatibility build is a future validation goal and is not supported by the current version.

## Local development

### Prerequisites

- Windows 10 or Windows 11
- Node.js 20 or newer
- pnpm 11.1.3 (the version used in CI)
- Rust stable with the MSVC toolchain
- The WebView2 and C++ build environment required by Tauri 2 on Windows

### Start the development app

```powershell
corepack enable
corepack prepare pnpm@11.1.3 --activate
pnpm install --frozen-lockfile
pnpm tauri dev
```

### Build the debug executable

Debug builds are the default artifact for development validation:

```powershell
pnpm tauri build --debug
```

The output is written to `src-tauri/target/debug/`. Tauri bundling is currently disabled, so this command produces the executable but not an official installer.

## Quality checks

Run these checks before submitting a change:

```powershell
pnpm check:node
pnpm check:source
pnpm typecheck
pnpm test
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo clippy --all-targets --locked --manifest-path src-tauri/Cargo.toml -- -D warnings
```

CI runs equivalent checks on a Windows runner. UI changes also require real-window, real-interaction, and multi-size screenshot validation from the same debug build.

## Repository layout

```text
src/                         React pages, shared components, client models, and quick surfaces
src-tauri/src/               Rust commands, services, managers, and Windows infrastructure
docs/prototypes/             Visual references and measurement notes
docs/architecture/           Architecture boundaries and the frontend design system
docs/audits/                 Real-device audit reports and reproducible evidence
scripts/                     Node checks, window capture, and development utilities
```

## Roadmap

Near-term work follows this order:

1. Finish clipboard and quick-launch validation across dark mode, high DPI, Windows versions, and exceptional focus scenarios.
2. Implement OpenDeskTools' own F1 region-capture pipeline.
3. Build F3 image pinning on the shared image and window infrastructure.
4. Complete installer, upgrade, compatibility, performance, and privacy checks before release.

The [page prototypes](./docs/prototypes/pages/) define the visual direction. The [development plan](./docs/development-plan.md) remains the source of truth for architecture and stage gates.

## Contributing

Issues that describe the use case, reproduction steps, and expected experience are welcome, as are focused and verifiable pull requests. For UI changes, include the affected entry points, states, window sizes, and real-runtime validation results.

## License

This project is licensed under the [PolyForm Noncommercial License 1.0.0](./LICENSE). The current license permits noncommercial use; commercial use is not licensed.
