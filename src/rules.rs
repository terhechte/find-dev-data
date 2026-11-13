//! Conservative directory signatures. No external tools or shell commands are run.
use serde::{Deserialize, Serialize};
use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Category {
    Dependencies,
    Build,
    Cache,
    Models,
}

impl Category {
    pub const ALL: [Self; 4] = [Self::Dependencies, Self::Build, Self::Cache, Self::Models];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dependencies => "Dependencies",
            Self::Build => "Build artifacts",
            Self::Cache => "Tool caches",
            Self::Models => "AI models",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Rule {
    pub tool: &'static str,
    pub category: Category,
    pub detail: &'static str,
}

fn rule(tool: &'static str, category: Category, detail: &'static str) -> Rule {
    Rule {
        tool,
        category,
        detail,
    }
}

pub fn classify(path: &Path, names: &[OsString]) -> Option<Rule> {
    use Category::*;
    let name = path.file_name()?.to_str()?;
    let parent = path.parent()?;
    let has = |name: &str| names.iter().any(|n| n == name);
    let suffix = |suffix: &str| path.ends_with(suffix);

    // Shared model stores remain one row: their blobs can serve many models.
    for (ending, tool, detail) in [
        (
            ".ollama/models",
            "Ollama",
            "Shared model blobs and manifests",
        ),
        (
            ".lmstudio/models",
            "LM Studio",
            "Downloaded GGUF and MLX models",
        ),
        (
            ".cache/lm-studio/models",
            "LM Studio",
            "Legacy model downloads",
        ),
        (".omlx/models", "oMLX", "Downloaded models"),
        (".omlx/cache", "oMLX cache", "SSD prompt / KV cache"),
        (
            "huggingface/hub",
            "Hugging Face / MLX",
            "Shared Hub cache; also used by MLX servers",
        ),
        (
            "huggingface/xet",
            "Hugging Face Xet",
            "Download chunk cache",
        ),
        (
            "huggingface/transformers",
            "Transformers",
            "Legacy model cache",
        ),
        (".cache/torch/hub", "PyTorch", "Downloaded model weights"),
        (".cache/whisper", "Whisper", "Speech model downloads"),
        (".cache/llama.cpp", "llama.cpp", "Downloaded GGUF models"),
        (
            "nomic.ai/GPT4All",
            "GPT4All",
            "Model store (may include app metadata)",
        ),
        ("Jan/data/models", "Jan", "Downloaded local models"),
        ("jan/models", "Jan", "Legacy local models"),
        ("Jan/models", "Jan", "Downloaded local models"),
        (
            "com.nomic.gpt4all/models",
            "GPT4All",
            "Downloaded local models",
        ),
    ] {
        if suffix(ending) {
            return Some(rule(tool, Models, detail));
        }
    }

    for (ending, tool, detail) in [
        (
            ".cargo/registry",
            "Cargo registry",
            "Crate archives, source checkouts and index",
        ),
        (".cargo/git", "Cargo Git", "Git dependencies and checkouts"),
        (".npm", "npm cache", "Package download cache and logs"),
        (".pnpm-store", "pnpm store", "Shared package store"),
        ("pnpm/store", "pnpm store", "Shared package store"),
        ("Caches/Yarn", "Yarn cache", "Cached package downloads"),
        (".cache/yarn", "Yarn cache", "Cached package downloads"),
        (".bun/install/cache", "Bun cache", "Shared package cache"),
        (
            "Caches/pip",
            "pip cache",
            "Python package downloads and wheels",
        ),
        (
            ".cache/pip",
            "pip cache",
            "Python package downloads and wheels",
        ),
        (
            "Caches/uv",
            "uv cache",
            "Python packages, wheels and interpreters",
        ),
        (
            ".cache/uv",
            "uv cache",
            "Python packages, wheels and interpreters",
        ),
        (
            "Caches/pypoetry",
            "Poetry cache",
            "Python packages and environments",
        ),
        (
            ".gradle/caches",
            "Gradle cache",
            "Shared dependency and build cache",
        ),
        (
            ".gradle/wrapper/dists",
            "Gradle distributions",
            "Downloaded Gradle versions",
        ),
        (
            ".m2/repository",
            "Maven cache",
            "Downloaded Java dependencies",
        ),
        (
            "Caches/Homebrew",
            "Homebrew cache",
            "Downloaded bottles and source archives",
        ),
        (
            ".pub-cache",
            "Dart / Flutter cache",
            "Downloaded Dart packages",
        ),
        (
            "Caches/org.swift.swiftpm",
            "SwiftPM cache",
            "Shared Swift package cache",
        ),
        ("Caches/sccache", "sccache", "Compiler cache"),
        ("Caches/ccache", "ccache", "Compiler cache"),
        (".cache/sccache", "sccache", "Compiler cache"),
        (".ccache", "ccache", "Compiler cache"),
    ] {
        if suffix(ending) {
            return Some(rule(tool, Cache, detail));
        }
    }

    if parent.file_name().is_some_and(|n| n == "DerivedData")
        && (has("Build") || has("Index.noindex") || has("info.plist"))
    {
        return Some(rule(
            "Xcode DerivedData",
            Build,
            "One project's derived build data and index",
        ));
    }

    match name {
        "node_modules" => {
            return Some(rule(
                "Node.js",
                Dependencies,
                "Installed npm, pnpm or Yarn dependencies",
            ));
        }
        "target" if parent.join("Cargo.toml").is_file() || has(".rustc_info.json") => {
            return Some(rule("Rust", Build, "Cargo build output"));
        }
        ".build" if parent.join("Package.swift").is_file() || has("workspace-state.json") => {
            return Some(rule(
                "SwiftPM",
                Build,
                "Swift package build output and dependencies",
            ));
        }
        ".venv" | "venv" | "env" | ".env" if has("pyvenv.cfg") || has("conda-meta") => {
            return Some(rule("Python", Dependencies, "Python virtual environment"));
        }
        "__pycache__" => return Some(rule("Python bytecode", Cache, "Compiled Python modules")),
        ".tox" | ".nox"
            if parent.join("pyproject.toml").is_file() || parent.join("tox.ini").is_file() =>
        {
            return Some(rule(
                "Python test environments",
                Dependencies,
                "Isolated test environments",
            ));
        }
        ".next" if parent.join("package.json").is_file() => {
            return Some(rule("Next.js", Build, "Next.js build output and cache"));
        }
        ".nuxt" | ".output" if parent.join("package.json").is_file() => {
            return Some(rule("Nuxt", Build, "Nuxt generated build output"));
        }
        ".turbo" => return Some(rule("Turborepo", Cache, "Task build cache")),
        ".dart_tool" => {
            return Some(rule(
                "Dart / Flutter",
                Build,
                "Generated package and build data",
            ));
        }
        "Pods" if parent.join("Podfile").is_file() => {
            return Some(rule(
                "CocoaPods",
                Dependencies,
                "Installed iOS / macOS dependencies",
            ));
        }
        "Carthage" if parent.join("Cartfile").is_file() => {
            return Some(rule(
                "Carthage",
                Dependencies,
                "Downloaded dependencies and built frameworks",
            ));
        }
        ".terraform" => {
            return Some(rule(
                "Terraform",
                Dependencies,
                "Downloaded providers and modules",
            ));
        }
        "build" | "Build" if has("CMakeCache.txt") => {
            return Some(rule("CMake", Build, "Configured CMake build tree"));
        }
        "build" | "Build" if has_xcode_project(parent) => {
            return Some(rule(
                "Xcode build",
                Build,
                "Build folder beside an Xcode project",
            ));
        }
        "build"
            if parent.join("build.gradle").is_file()
                || parent.join("build.gradle.kts").is_file() =>
        {
            return Some(rule("Gradle", Build, "Project build output"));
        }
        _ => {}
    }

    if name.starts_with("models--") && has("blobs") && has("snapshots") {
        return Some(rule(
            "Hugging Face / MLX",
            Models,
            "Cached Hugging Face model repository",
        ));
    }
    if has("blobs") && has("manifests") {
        return Some(rule(
            "Ollama",
            Models,
            "Model store identified by blobs and manifests",
        ));
    }
    if names
        .iter()
        .any(|name| is_model_file(&name.to_string_lossy()))
    {
        return Some(rule(
            "Local model files",
            Models,
            "Folder containing GGUF, safetensors, ONNX or model weights",
        ));
    }
    None
}

fn has_xcode_project(parent: &Path) -> bool {
    fs::read_dir(parent)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|ext| ext == "xcodeproj" || ext == "xcworkspace")
        })
}

pub fn is_model_file(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.ends_with(".gguf")
        || name.ends_with(".safetensors")
        || name.ends_with(".onnx")
        || (name.starts_with("pytorch_model") && name.ends_with(".bin"))
}

/// Fast paths are visited before general discovery, but only within the chosen root.
/// Environment overrides are additive: old downloads may still exist at defaults.
pub fn known_locations(home: &Path) -> Vec<(PathBuf, Rule)> {
    use Category::*;
    let mut paths = Vec::new();
    for relative in [
        ".cargo/registry",
        ".cargo/git",
        ".npm",
        ".pnpm-store",
        "Library/pnpm/store",
        "Library/Caches/Yarn",
        ".cache/yarn",
        ".bun/install/cache",
        "Library/Caches/pip",
        "Library/Caches/uv",
        "Library/Caches/pypoetry",
        ".gradle/caches",
        ".gradle/wrapper/dists",
        ".m2/repository",
        "Library/Caches/Homebrew",
        ".pub-cache",
        "Library/Caches/org.swift.swiftpm",
        ".ollama/models",
        ".lmstudio/models",
        ".cache/lm-studio/models",
        ".omlx/models",
        ".omlx/cache",
        ".cache/huggingface/hub",
        ".cache/huggingface/xet",
        ".cache/huggingface/transformers",
        ".cache/torch/hub",
        ".cache/whisper",
        ".cache/llama.cpp",
        "Library/Application Support/nomic.ai/GPT4All",
        "Library/Application Support/Jan/data/models",
        "jan/models",
    ] {
        let path = home.join(relative);
        if let Some(rule) = classify(&path, &[]) {
            paths.push((path, rule));
        }
    }
    for (key, tool, category) in [
        ("OLLAMA_MODELS", "Ollama", Models),
        ("OMLX_MODEL_DIR", "oMLX", Models),
        ("HF_HUB_CACHE", "Hugging Face / MLX", Models),
        ("HUGGINGFACE_HUB_CACHE", "Hugging Face / MLX", Models),
        ("HF_XET_CACHE", "Hugging Face Xet", Models),
        ("TRANSFORMERS_CACHE", "Transformers", Models),
        ("CARGO_TARGET_DIR", "Rust", Build),
        ("UV_CACHE_DIR", "uv cache", Cache),
        ("PIP_CACHE_DIR", "pip cache", Cache),
    ] {
        if let Some(value) = env::var_os(key).filter(|v| !v.is_empty()) {
            paths.push((
                expand_home(Path::new(&value), home),
                rule(tool, category, "Configured environment path"),
            ));
        }
    }
    if let Some(cargo) = env::var_os("CARGO_HOME").filter(|v| !v.is_empty()) {
        let cargo = expand_home(Path::new(&cargo), home);
        paths.push((
            cargo.join("registry"),
            rule("Cargo registry", Cache, "Configured Cargo registry"),
        ));
        paths.push((
            cargo.join("git"),
            rule("Cargo Git", Cache, "Configured Cargo Git cache"),
        ));
    }
    let hf_home = env::var_os("HF_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".cache"))
                .join("huggingface")
        });
    let hf_home = expand_home(&hf_home, home);
    paths.push((
        hf_home.join("hub"),
        rule(
            "Hugging Face / MLX",
            Models,
            "Shared Hub cache; also used by MLX servers",
        ),
    ));
    paths.push((
        hf_home.join("xet"),
        rule("Hugging Face Xet", Models, "Download chunk cache"),
    ));

    // oMLX persists custom model roots in its small JSON settings file.
    let settings = home.join(".omlx/settings.json");
    if fs::metadata(&settings).is_ok_and(|m| m.len() < 1024 * 1024)
        && let Ok(data) = fs::read(&settings)
        && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&data)
    {
        for pointer in ["/model/model_dir", "/model_dir"] {
            if let Some(path) = value
                .pointer(pointer)
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                paths.push((
                    expand_home(Path::new(path), home),
                    rule("oMLX", Models, "Model directory from oMLX settings"),
                ));
            }
        }
    }
    paths
}

pub fn expand_home(path: &Path, home: &Path) -> PathBuf {
    if path == Path::new("~") {
        home.to_path_buf()
    } else if let Ok(rest) = path.strip_prefix("~") {
        home.join(rest)
    } else {
        path.to_path_buf()
    }
}
