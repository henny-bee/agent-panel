//! The Languages bar of GitHub's repository page: bytes of code per language
//! across the checkout's tracked and new files. Like GitHub, it counts only
//! programming and markup languages, and skips vendored code, docs and
//! minified bundles.
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct Share {
    pub name: &'static str,
    /// GitHub's colour for the language, as 0xRRGGBB.
    pub color: u32,
    pub bytes: u64,
    /// A file with this language's extension, for its file-type icon.
    pub sample: &'static str,
}

const OTHER: &str = "Other";

/// The biggest `rows` languages, then one "Other" share with the rest.
pub fn languages(cwd: &str, rows: usize) -> Result<Vec<Share>, String> {
    if crate::files::remote::is_remote(Path::new(cwd)) {
        return Err("Languages aren’t available for remote projects yet.".into());
    }
    // ponytail: the listing stops at 20,000 files, so a bigger checkout is
    // measured on its first 20,000; walk the rest if that ever matters.
    let files = crate::files::list_project_files(cwd)?;
    Ok(summarize(
        files.into_iter().filter_map(|file| {
            let (_, sample) = language(&file.relative)?;
            if skipped(&file.relative) {
                return None;
            }
            let meta = std::fs::symlink_metadata(&file.path).ok()?;
            if !meta.is_file() || (matches!(sample, "x.js" | "x.css") && minified(&file.path)) {
                return None;
            }
            Some((file.relative, meta.len()))
        }),
        rows,
    ))
}

/// Groups `(relative path, bytes)` by language, biggest first.
pub fn summarize(files: impl IntoIterator<Item = (String, u64)>, rows: usize) -> Vec<Share> {
    let mut shares: Vec<Share> = Vec::new();
    for (path, bytes) in files {
        let Some(((name, color), sample)) = language(&path) else {
            continue;
        };
        match shares.iter_mut().find(|share| share.name == name) {
            Some(share) => share.bytes += bytes,
            None => shares.push(Share {
                name,
                color,
                bytes,
                sample,
            }),
        }
    }
    shares.retain(|share| share.bytes > 0);
    shares.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.name.cmp(b.name)));
    if shares.len() > rows {
        let bytes = shares.drain(rows..).map(|share| share.bytes).sum();
        shares.push(Share {
            name: OTHER,
            color: 0x8b949e,
            bytes,
            sample: "",
        });
    }
    shares
}

/// Vendored code, docs and generated files, as GitHub's Linguist skips them.
fn skipped(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let dirs = lower.rsplit_once('/').map_or("", |(dirs, _)| dirs);
    dirs.split('/').any(|dir| {
        matches!(
            dir,
            "vendor"
                | "vendors"
                | "third_party"
                | "third-party"
                | "bower_components"
                | "pods"
                | "carthage"
                | ".yarn"
                | "__generated__"
        )
    }) || lower.starts_with("doc/")
        || lower.starts_with("docs/")
        || ["documentation", "examples", "example", "samples", "sample"]
            .iter()
            .any(|dir| dirs.split('/').any(|d| d == *dir))
        || [
            ".min.js",
            ".min.css",
            "-min.js",
            ".pb.go",
            "_pb2.py",
            ".g.dart",
            ".designer.cs",
        ]
        .iter()
        .any(|end| lower.ends_with(end))
        || lower.contains(".generated.")
}

/// Linguist's test for a bundled script or stylesheet: lines averaging over
/// 110 characters.
fn minified(path: &str) -> bool {
    let Ok(text) = std::fs::read(path) else {
        return false;
    };
    let lines = text.split(|b| *b == b'\n').count().max(1);
    text.len() / lines > 110
}

/// The language of a path, and a file name that shows its file-type icon.
fn language(path: &str) -> Option<((&'static str, u32), &'static str)> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let by_name = match name {
        "Makefile" | "makefile" | "GNUmakefile" => Some(("Makefile", 0x427819, "Makefile")),
        "Dockerfile" => Some(("Dockerfile", 0x384d54, "Dockerfile")),
        "CMakeLists.txt" => Some(("CMake", 0xda3434, "CMakeLists.txt")),
        _ => None,
    };
    if let Some((language, color, matched)) = by_name {
        return Some(((language, color), matched));
    }
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    let (language, color, sample): (&str, u32, &str) = match ext.as_str() {
        "rs" => ("Rust", 0xdea584, "x.rs"),
        "ts" | "tsx" | "mts" | "cts" => ("TypeScript", 0x3178c6, "x.ts"),
        "js" | "jsx" | "mjs" | "cjs" => ("JavaScript", 0xf1e05a, "x.js"),
        "swift" => ("Swift", 0xf05138, "x.swift"),
        "kt" | "kts" => ("Kotlin", 0xa97bff, "x.kt"),
        "java" => ("Java", 0xb07219, "x.java"),
        "py" | "pyi" | "pyw" => ("Python", 0x3572a5, "x.py"),
        "go" => ("Go", 0x00add8, "x.go"),
        "c" | "h" => ("C", 0x555555, "x.c"),
        "cpp" | "cc" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "h++" => ("C++", 0xf34b7d, "x.cpp"),
        "cs" => ("C#", 0x178600, "x.cs"),
        "m" => ("Objective-C", 0x438eff, "x.m"),
        "mm" => ("Objective-C++", 0x6866fb, "x.mm"),
        "rb" => ("Ruby", 0x701516, "x.rb"),
        "php" => ("PHP", 0x4f5d95, "x.php"),
        "sh" | "bash" | "zsh" => ("Shell", 0x89e051, "x.sh"),
        "ps1" | "psm1" | "psd1" => ("PowerShell", 0x012456, "x.ps1"),
        "bat" | "cmd" => ("Batchfile", 0xc1f12e, "x.bat"),
        "html" | "htm" => ("HTML", 0xe34c26, "x.html"),
        "css" => ("CSS", 0x663399, "x.css"),
        "scss" => ("SCSS", 0xc6538c, "x.scss"),
        "sass" => ("Sass", 0xa53b70, "x.sass"),
        "less" => ("Less", 0x1d365d, "x.less"),
        "vue" => ("Vue", 0x41b883, "x.vue"),
        "svelte" => ("Svelte", 0xff3e00, "x.svelte"),
        "astro" => ("Astro", 0xff5a03, "x.astro"),
        "dart" => ("Dart", 0x00b4ab, "x.dart"),
        "lua" => ("Lua", 0x000080, "x.lua"),
        "zig" => ("Zig", 0xec915c, "x.zig"),
        "scala" => ("Scala", 0xc22d40, "x.scala"),
        "ex" | "exs" => ("Elixir", 0x6e4a7e, "x.ex"),
        "erl" => ("Erlang", 0xb83998, "x.erl"),
        "hs" => ("Haskell", 0x5e5086, "x.hs"),
        "ml" => ("OCaml", 0xef7a08, "x.ml"),
        "fs" | "fsx" => ("F#", 0xb845fc, "x.fs"),
        "clj" | "cljs" | "cljc" => ("Clojure", 0xdb5855, "x.clj"),
        "r" => ("R", 0x198ce7, "x.r"),
        "jl" => ("Julia", 0xa270ba, "x.jl"),
        "pl" | "pm" => ("Perl", 0x0298c3, "x.pl"),
        "nim" => ("Nim", 0xffc200, "x.nim"),
        "elm" => ("Elm", 0x60b5cc, "x.elm"),
        "gleam" => ("Gleam", 0xffaff3, "x.gleam"),
        "vb" => ("Visual Basic .NET", 0x945db7, "x.vb"),
        "sol" => ("Solidity", 0xaa6746, "x.sol"),
        "asm" | "s" => ("Assembly", 0x6e4c13, "x.asm"),
        "glsl" | "vert" | "frag" => ("GLSL", 0x5686a5, "x.glsl"),
        "wgsl" => ("WGSL", 0x1a5e9a, "x.wgsl"),
        "metal" => ("Metal", 0x8f14e9, "x.metal"),
        "hlsl" => ("HLSL", 0xaace60, "x.hlsl"),
        "tf" | "hcl" => ("HCL", 0x844fba, "x.tf"),
        "nix" => ("Nix", 0x7e7eff, "x.nix"),
        "vim" => ("Vim Script", 0x199f4b, "x.vim"),
        "el" => ("Emacs Lisp", 0xc065db, "x.el"),
        "tex" => ("TeX", 0x3d6117, "x.tex"),
        "ipynb" => ("Jupyter Notebook", 0xda5b0b, "x.ipynb"),
        _ => return None,
    };
    Some(((language, color), sample))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, bytes: u64) -> (String, u64) {
        (path.into(), bytes)
    }

    #[test]
    fn groups_by_language_biggest_first_with_the_rest_as_other() {
        let shares = summarize(
            [
                file("src/main.rs", 700),
                file("src/lib.rs", 100),
                file("app/index.ts", 150),
                file("app/view.tsx", 50),
                file("ios/App.swift", 30),
                file("web/a.js", 20),
                file("web/b.py", 10),
                file("README.md", 5_000),
                file("Cargo.toml", 900),
                file("data.json", 900),
            ],
            3,
        );
        let got: Vec<_> = shares.iter().map(|s| (s.name, s.bytes)).collect();
        assert_eq!(
            got,
            [
                ("Rust", 800),
                ("TypeScript", 200),
                ("Swift", 30),
                ("Other", 30)
            ],
            "Markdown, TOML and JSON aren't code; .ts and .tsx are both TypeScript"
        );
        assert_eq!(shares[0].color, 0xdea584);
        assert_eq!(shares[1].sample, "x.ts");
    }

    #[test]
    fn no_other_row_when_everything_fits() {
        let shares = summarize([file("a.rs", 1), file("b.go", 2)], 6);
        assert_eq!(
            shares.iter().map(|s| s.name).collect::<Vec<_>>(),
            ["Go", "Rust"]
        );
    }

    #[test]
    fn vendored_docs_and_generated_files_are_skipped() {
        for path in [
            "vendor/gpui-pre/src/lib.rs",
            "crates/x/third_party/a.c",
            "docs/native/examples/visual-scene.js",
            "web/app.min.js",
            "api/service.pb.go",
            "lib/model.g.dart",
            "src/types.generated.ts",
            "packages/a/examples/demo.ts",
        ] {
            assert!(skipped(path), "{path}");
        }
        for path in ["src/main.rs", "crates/vendored/src/lib.rs", "docsite/a.ts"] {
            assert!(!skipped(path), "{path}");
        }
    }

    #[test]
    fn minified_bundles_are_skipped_but_normal_scripts_count() {
        let dir = std::env::temp_dir().join(format!("monocode-languages-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bundle = dir.join("bundle.js");
        std::fs::write(&bundle, format!("var a={};\n", "1,".repeat(400))).unwrap();
        let script = dir.join("script.js");
        std::fs::write(&script, "const a = 1;\nconsole.log(a);\n").unwrap();
        assert!(minified(&bundle.to_string_lossy()));
        assert!(!minified(&script.to_string_lossy()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn makefiles_and_dockerfiles_go_by_name() {
        let shares = summarize([file("Makefile", 3), file("deploy/Dockerfile", 2)], 6);
        assert_eq!(
            shares.iter().map(|s| s.name).collect::<Vec<_>>(),
            ["Makefile", "Dockerfile"]
        );
    }
}
