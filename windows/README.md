# Multi-Finder for Windows

Windows 10/11 (x64) version of Multi-Finder — the same five screens as the macOS app
(All Folders, Duplicates, Compare Folders, Categories, App Leftovers), built with Tauri v2
(Rust + WebView2). Design notes and the backend ⇄ UI contract: [SPEC.md](SPEC.md),
[src/api/types.ts](src/api/types.ts).

```
core/        mf-core — all logic (index, scanner, watcher, duplicates, compare, Recycle Bin + undo,
             text extraction, local AI categories via llama.cpp, app leftovers); tests run on Linux too
src-tauri/   the Tauri app: commands and events that connect mf-core to the UI
src/         React + Fluent UI frontend; src/api/mock.ts lets every screen run in a plain browser
tools/       gen-i18n.mjs (strings from ../Packaging/Localizable.xcstrings), shots.mjs, smoke.ps1
```

Build on Windows (Node 22, Rust stable, Visual Studio C++ build tools, CMake):

```sh
npm ci
cargo test -p mf-core --release
npx tauri build        # target/release/multi-finder.exe + target/release/bundle/nsis/*-setup.exe
```

`npm run dev` shows the UI in a browser with the mock backend. The data folder is
`%LOCALAPPDATA%\com.taeholim.multifinder`; set `MF_DATA_DIR` to use another one (tests, screenshots).
