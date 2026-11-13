# Find Dev Data

A native macOS desktop app written in Rust with **egui / eframe**. Find developer dependencies, build artifacts, tool caches, and local AI models, with a size for each folder.

## Run

Requires macOS, Rust 1.92 or later, and the Xcode Command Line Tools.

```sh
cargo run --release
```

Create a double-clickable, locally signed app bundle:

```sh
bash scripts/bundle-macos.sh
open "dist/Find Dev Data.app"
```

The bundle script uses Apple's built-in Swift, AppKit, and `codesign`; it requires no packaging tools. It builds for your Mac's architecture. For a distributable build, provide a Developer ID signature and notarization separately.

## Use

Choose **Entire disk**, **Home folder**, or a custom folder, then **Start scan**. The interface stays responsive while a background thread searches and measures folders. Results stream in as each measurement finishes. Stop a scan at any time; its partial results remain available.

- Filter by category, path or tool, and minimum size; press **⌘F** to search.
- Click Folder, Tool, or Size to sort. The list virtualizes rows for large result sets.
- Switch between **File size** and **Space on disk**.
- Select a row for both sizes, file count, and measurement notes.
- Reveal a folder in Finder, or copy its full path from the context menu.
- Export the visible, sorted results as CSV, including raw byte counts and completion status.
- Scan settings and window geometry persist between launches. Scan results do not.

**Full Disk Access:** macOS restricts some locations even for an unsandboxed app. Add the built app under **System Settings → Privacy & Security → Full Disk Access**, restart it, and scan again. When running through `cargo`, grant access to the launching terminal. Access issues are counted and the first 200 are available in the app. Unreadable files produce partial sizes; a scan is never a guarantee that every location was readable.

## Detected folders

| Area | Detection |
| --- | --- |
| JavaScript | `node_modules`, npm, pnpm, Yarn and Bun caches; Next.js, Nuxt, Turborepo build data |
| Rust | `target` beside `Cargo.toml` or containing `.rustc_info.json`; Cargo registry and Git caches |
| Swift / Xcode | SwiftPM `.build`, `Build` / `build` beside Xcode projects, each DerivedData project, SwiftPM shared cache, CocoaPods and Carthage |
| Python | Virtual environments with `pyvenv.cfg` or `conda-meta`, `__pycache__`, tox / nox environments, pip, uv and Poetry caches |
| Other tools | CMake build trees, Gradle project output and caches, Maven repository, Homebrew downloads, Dart / Flutter, Terraform, sccache and ccache |
| LM Studio | `~/.lmstudio/models`, legacy `~/.cache/lm-studio/models` |
| Hugging Face / MLX servers | Hub, Xet and legacy Transformers caches; `models--…` repositories with `blobs` and `snapshots` |
| Ollama | `~/.ollama/models`, and custom stores identified by `blobs` and `manifests` |
| oMLX | `~/.omlx/models`, `~/.omlx/cache`, and custom model directories from `~/.omlx/settings.json` |
| Other model apps | GPT4All, Jan (current and legacy stores), PyTorch Hub, Whisper, llama.cpp |
| Custom model folders | Folders containing `.gguf`, `.safetensors`, `.onnx`, or `pytorch_model*.bin` files anywhere in the selected scope |

Generic names like `target`, `.build`, `venv`, and `Build` need project or environment markers. A custom model folder's entire contents are measured, including metadata or unrelated files in that same folder. Shared model stores appear once because model blobs can be shared across multiple models.

Known locations are checked first, within the selected root. These inherited environment overrides are also recognized: `CARGO_HOME`, `CARGO_TARGET_DIR`, `OLLAMA_MODELS`, `OMLX_MODEL_DIR`, `HF_HOME`, `XDG_CACHE_HOME`, `HF_HUB_CACHE`, `HUGGINGFACE_HUB_CACHE`, `HF_XET_CACHE`, `TRANSFORMERS_CACHE`, `UV_CACHE_DIR`, and `PIP_CACHE_DIR`. Defaults are checked too, since old downloads may remain there. Finder-launched apps do not inherit variables from your shell; choose a custom folder for other configured stores. There is no universal model registry, and arbitrary renamed stores without recognizable weight files may not be found.

Default locations follow upstream documentation: [LM Studio](https://lmstudio.ai/docs/app/advanced/import-model), [Hugging Face](https://huggingface.co/docs/huggingface_hub/en/package_reference/environment_variables), [Ollama](https://docs.ollama.com/faq#where-are-models-stored), [oMLX](https://github.com/jundot/omlx), [GPT4All](https://github.com/nomic-ai/gpt4all/wiki/Frequently-Asked-Questions), and [Jan](https://jan.ai/docs/data-folder).

## Measurement semantics

- This app only reads your disk; it does not delete files, execute project code, invoke package managers, or contact model services.
- File size sums regular-file lengths. Space on disk sums allocated blocks (`st_blocks × 512`). Directory metadata is not included.
- Symlinks are not followed or added to sizes. A selected root or configured store root is canonicalized once, allowing a relocated top-level store to be measured if it is within the selected scope.
- Hard-linked files count once within a finding. Hard links shared across separate rows and APFS clones cannot be attributed uniquely, so totals can overstate physical usage. These numbers are **not guaranteed reclaimable space**.
- Nested matches roll into their outer finding, preventing overlapping directory totals. For example, a dependency's nested `node_modules` is included in the project's top-level row.
- Discovery skips virtual filesystems, `.git` internals, Spotlight indexes, trash and Time Machine metadata, and the macOS duplicate Data-volume view at `/System/Volumes`. Hidden folders elsewhere are included. Enable **External volumes** to include `/Volumes`; choosing an external folder directly also works.
- Permission failures, disappearing files, and cancellation produce partial data. Files changing during a scan can make counts differ from a later Finder measurement. Inaccessible locations cannot be measured.
- The app reads file metadata and directory listings, plus the small oMLX settings file to locate configured models; it does not open model weights.

## Headless scanning

The same scanner can be used without a window. Diagnostics go to stderr; CSV goes to stdout or a chosen file.

```sh
cargo run --release -- --scan ~/Developer --csv /tmp/dev-data.csv
cargo run --release -- --scan / --include-volumes --csv /tmp/all-dev-data.csv
```

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
bash scripts/bundle-macos.sh --debug
```

Filesystem integration tests exercise detection, false-positive protection, per-project DerivedData, nested-store deduplication, cancellation, scope boundaries, symlink loops, hard links, permission errors, and CSV quoting.

## License

Licensed under the [MIT License](LICENSE).

