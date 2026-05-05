# OmniSync — App Workspace

Tauri v2 + React 19 + TypeScript + Rust desktop app. This directory is the build root.

> For project overview, features, install instructions and screenshots see the [root README](../README.md).

## Layout

| Path             | Purpose                                                     |
| ---------------- | ----------------------------------------------------------- |
| `src/`           | React + TypeScript frontend (Vite, Tailwind v4, Framer Motion) |
| `src-tauri/`     | Rust backend (Tauri commands, Telegram MTProto, Drive sync) |
| `public/`        | Static assets bundled into the WebView                      |
| `index.html`     | Vite entry HTML                                             |
| `vite.config.ts` | Vite + Tauri dev-server configuration                       |

## Scripts

```bash
npm install              # install JS deps
npm run dev              # vite dev server only (frontend, no Tauri shell)
npm run build            # tsc + vite build (frontend production bundle)
npm run tauri dev        # full app in dev mode (recommended)
npm run tauri build      # production binary + installers
```

## Rust workspace

Inside `src-tauri/`:

```bash
cargo check              # quick type-check
cargo clippy --all-targets -- -D warnings
cargo build --release
```

## Recommended IDE Setup

[VS Code](https://code.visualstudio.com/) with:
- [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode)
- [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
