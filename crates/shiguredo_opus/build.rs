use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Command,
};

use bindgen::callbacks::{ItemInfo, ItemKind, ParseCallbacks};
use cmake::Config;

// 依存ライブラリの名前
const LIB_NAME: &str = "opus";
const LINK_NAME: &str = "opus";
const SHIGUREDO_OPUS_SYMBOL_PREFIX: &str = "shiguredo_opus_";

fn main() {
    // Cargo.toml か build.rs が更新されたら、依存ライブラリを再ビルドする
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rerun-if-changed=build.rs");

    // 各種変数やビルドディレクトリのセットアップ
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("infallible"));
    let out_source_dir = out_dir.join("source/");
    let src_dir = out_source_dir.join(LIB_NAME);
    let output_metadata_path = out_dir.join("metadata.rs");
    let output_bindings_path = out_dir.join("bindings.rs");
    let output_symbol_rename_map_path = out_dir.join("symbol_rename_map.txt");

    let _ = std::fs::remove_dir_all(&out_source_dir);
    std::fs::create_dir(&out_source_dir).expect("failed to create source directory");

    // 各種メタデータを書き込む
    let (git_url, version) = get_git_url_and_version();
    std::fs::write(
        output_metadata_path,
        format!(
            concat!(
                "pub const BUILD_METADATA_REPOSITORY: &str={:?};\n",
                "pub const BUILD_METADATA_VERSION: &str={:?};\n",
            ),
            git_url, version
        ),
    )
    .expect("failed to write metadata file");

    if std::env::var("DOCS_RS").is_ok() {
        // Docs.rs 向けのビルドでは git clone ができないので build.rs の処理はスキップして、
        // 代わりに、ドキュメント生成時に最低限必要な構造体だけをダミーで出力している。
        //
        // See also: https://docs.rs/about/builds
        std::fs::write(
            output_bindings_path,
            "pub struct OpusEncoder; pub struct OpusDecoder;",
        )
        .expect("write file error");
        return;
    }

    // 依存ライブラリのリポジトリを取得する
    git_clone_external_lib(&out_source_dir);

    // 依存ライブラリを CMake でビルドする
    let dst = Config::new(&src_dir)
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("OPUS_BUILD_SHARED_LIBRARY", "OFF")
        .define("OPUS_BUILD_TESTING", "OFF")
        .define("OPUS_BUILD_PROGRAMS", "OFF")
        .profile("Release")
        .build();

    let input_header_path = src_dir.join("include/opus.h");
    let output_lib_dir = dst.join("lib");
    let static_library_path = find_static_library_path(&dst);

    let llvm_tools = discover_llvm_tools();
    let raw_symbols = collect_defined_external_symbols(&llvm_tools.nm, &static_library_path);
    let symbol_maps = build_symbol_rename_maps(raw_symbols, target_is_macos());

    if symbol_maps.raw_to_raw_renamed.is_empty() {
        panic!("no symbols were collected for symbol rewriting");
    }

    write_objcopy_rename_map(
        &output_symbol_rename_map_path,
        &symbol_maps.raw_to_raw_renamed,
    );

    rewrite_archive_symbols(
        &llvm_tools.objcopy,
        &static_library_path,
        &output_symbol_rename_map_path,
    );

    verify_symbol_rename(
        &llvm_tools.nm,
        &static_library_path,
        &symbol_maps.raw_to_raw_renamed,
    );

    // バインディングを生成する
    let callbacks = SymbolLinkNameCallbacks {
        canonical_rename_map: symbol_maps.canonical_to_canonical_renamed,
    };
    bindgen::Builder::default()
        .header(input_header_path.to_str().expect("invalid header path"))
        .parse_callbacks(Box::new(callbacks))
        .generate()
        .expect("failed to generate bindings")
        .write_to_file(output_bindings_path)
        .expect("failed to write bindings");

    println!(
        "cargo::rustc-link-search=native={}",
        output_lib_dir.display()
    );
    println!("cargo::rustc-link-lib=static={LINK_NAME}");
}

#[derive(Debug)]
struct SymbolLinkNameCallbacks {
    canonical_rename_map: BTreeMap<String, String>,
}

impl ParseCallbacks for SymbolLinkNameCallbacks {
    fn generated_link_name_override(&self, item_info: ItemInfo<'_>) -> Option<String> {
        match item_info.kind {
            ItemKind::Function | ItemKind::Var => {
                self.canonical_rename_map.get(item_info.name).cloned()
            }
            _ => None,
        }
    }
}

#[derive(Debug)]
struct LlvmTools {
    nm: PathBuf,
    objcopy: PathBuf,
}

#[derive(Debug)]
struct SymbolRenameMaps {
    raw_to_raw_renamed: BTreeMap<String, String>,
    canonical_to_canonical_renamed: BTreeMap<String, String>,
}

// 外部ライブラリのリポジトリを git clone する
fn git_clone_external_lib(build_dir: &Path) {
    let (git_url, version) = get_git_url_and_version();
    let success = Command::new("git")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg("--branch")
        .arg(version)
        .arg(git_url)
        .current_dir(build_dir)
        .status()
        .is_ok_and(|status| status.success());
    if !success {
        panic!("failed to clone {LIB_NAME} repository");
    }
}

// Cargo.toml から依存ライブラリの Git URL とバージョンタグを取得する
fn get_git_url_and_version() -> (String, String) {
    let cargo_toml: toml::Value =
        toml::from_str(include_str!("Cargo.toml")).expect("failed to parse Cargo.toml");
    if let Some((Some(git_url), Some(version))) = cargo_toml
        .get("package")
        .and_then(|v| v.get("metadata"))
        .and_then(|v| v.get("external-dependencies"))
        .and_then(|v| v.get(LIB_NAME))
        .map(|v| {
            (
                v.get("git").and_then(|s| s.as_str()),
                v.get("version").and_then(|s| s.as_str()),
            )
        })
    {
        (git_url.to_string(), version.to_string())
    } else {
        panic!(
            "Cargo.toml does not contain a valid [package.metadata.external-dependencies.{LIB_NAME}] table"
        );
    }
}

// CMake の出力ディレクトリから静的ライブラリの実体を探す
fn find_static_library_path(dst: &Path) -> PathBuf {
    let primary_candidates = [
        dst.join("lib/libopus.a"),
        dst.join("lib/opus.lib"),
        dst.join("lib64/libopus.a"),
        dst.join("lib64/opus.lib"),
    ];

    for path in primary_candidates {
        if path.is_file() {
            return path;
        }
    }

    panic!("failed to find static opus library under {}", dst.display());
}

// rustup の llvm-tools から llvm-nm / llvm-objcopy のパスを解決する
fn discover_llvm_tools() -> LlvmTools {
    let host = std::env::var("HOST").expect("HOST environment variable is not set");
    let sysroot = get_rustc_sysroot();
    let bin_dir = sysroot.join("lib").join("rustlib").join(host).join("bin");

    let nm = bin_dir.join(exe_name("llvm-nm"));
    let objcopy = bin_dir.join(exe_name("llvm-objcopy"));

    if !nm.is_file() || !objcopy.is_file() {
        panic!(concat!(
            "llvm tools were not found in Rust sysroot. ",
            "Please install them with: rustup component add llvm-tools"
        ));
    }

    LlvmTools { nm, objcopy }
}

// rustc --print sysroot の結果を取得する
fn get_rustc_sysroot() -> PathBuf {
    let output = Command::new("rustc")
        .arg("--print")
        .arg("sysroot")
        .output()
        .expect("failed to execute rustc --print sysroot");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("failed to resolve rust sysroot: {stderr}");
    }

    let sysroot =
        String::from_utf8(output.stdout).expect("rustc sysroot output is not valid UTF-8");
    PathBuf::from(sysroot.trim())
}

// 実行ファイル名をプラットフォームに応じて組み立てる
fn exe_name(base: &str) -> String {
    if std::env::consts::OS == "windows" {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

// 対象プラットフォームが macOS かどうかを判定する
fn target_is_macos() -> bool {
    std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "macos")
}

// 静的ライブラリから定義済み外部シンボルを収集する
fn collect_defined_external_symbols(nm_path: &Path, archive_path: &Path) -> BTreeSet<String> {
    let output = Command::new(nm_path)
        .arg("--defined-only")
        .arg("--extern-only")
        .arg("--format=just-symbols")
        .arg(archive_path)
        .output()
        .unwrap_or_else(|_| panic!("failed to run llvm-nm: {}", nm_path.display()));

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("llvm-nm failed for {}: {stderr}", archive_path.display());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut symbols = BTreeSet::new();
    for line in stdout.lines() {
        let symbol = line.trim();
        if symbol.is_empty() {
            continue;
        }
        if is_c_identifier(symbol) {
            symbols.insert(symbol.to_string());
        }
    }
    symbols
}

// raw シンボル名（objcopy 用）と canonical シンボル名（bindgen 用）のリネームマップを生成する
fn build_symbol_rename_maps(raw_symbols: BTreeSet<String>, is_macos: bool) -> SymbolRenameMaps {
    let mut raw_to_raw_renamed = BTreeMap::new();
    let mut canonical_to_canonical_renamed = BTreeMap::new();

    for raw_symbol in raw_symbols {
        let canonical_symbol = canonical_symbol_name(&raw_symbol, is_macos);

        if !is_c_identifier(&canonical_symbol) {
            continue;
        }
        if canonical_symbol.starts_with(SHIGUREDO_OPUS_SYMBOL_PREFIX) {
            continue;
        }

        let canonical_renamed_symbol = format!("{SHIGUREDO_OPUS_SYMBOL_PREFIX}{canonical_symbol}");
        let raw_renamed_symbol = if is_macos && raw_symbol.starts_with('_') {
            format!("_{canonical_renamed_symbol}")
        } else {
            canonical_renamed_symbol.clone()
        };

        raw_to_raw_renamed.insert(raw_symbol, raw_renamed_symbol);

        if let Some(previous) = canonical_to_canonical_renamed
            .insert(canonical_symbol.clone(), canonical_renamed_symbol.clone())
            && previous != canonical_renamed_symbol
        {
            panic!("duplicate canonical symbol mapping detected for {canonical_symbol}");
        }
    }

    SymbolRenameMaps {
        raw_to_raw_renamed,
        canonical_to_canonical_renamed,
    }
}

// macOS の装飾付きシンボルを bindgen 用の canonical 名に変換する
fn canonical_symbol_name(raw_symbol: &str, is_macos: bool) -> String {
    if is_macos {
        raw_symbol
            .strip_prefix('_')
            .unwrap_or(raw_symbol)
            .to_string()
    } else {
        raw_symbol.to_string()
    }
}

// objcopy の --redefine-syms で使うマップファイルを書き出す
fn write_objcopy_rename_map(path: &Path, raw_to_raw_renamed: &BTreeMap<String, String>) {
    let mut lines = String::new();
    for (raw_symbol, raw_renamed_symbol) in raw_to_raw_renamed {
        lines.push_str(raw_symbol);
        lines.push(' ');
        lines.push_str(raw_renamed_symbol);
        lines.push('\n');
    }
    std::fs::write(path, lines).expect("failed to write symbol rename map");
}

// 静的ライブラリのシンボルを直接書き換える
fn rewrite_archive_symbols(objcopy_path: &Path, archive_path: &Path, map_path: &Path) {
    let output_archive_path = archive_path.with_extension("renamed.tmp");

    let output = Command::new(objcopy_path)
        .arg("--redefine-syms")
        .arg(map_path)
        .arg(archive_path)
        .arg(&output_archive_path)
        .output()
        .unwrap_or_else(|_| panic!("failed to run llvm-objcopy: {}", objcopy_path.display()));

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "llvm-objcopy failed for {}: {stderr}",
            archive_path.display()
        );
    }

    std::fs::remove_file(archive_path).unwrap_or_else(|_| {
        panic!(
            "failed to remove original archive before replacement: {}",
            archive_path.display()
        )
    });
    std::fs::rename(&output_archive_path, archive_path).unwrap_or_else(|_| {
        panic!(
            "failed to replace archive with rewritten file: {}",
            archive_path.display()
        )
    });
}

// シンボル書き換え後に、元名が残っていないことと新名が存在することを検証する
fn verify_symbol_rename(
    nm_path: &Path,
    archive_path: &Path,
    raw_to_raw_renamed: &BTreeMap<String, String>,
) {
    let rewritten_symbols = collect_defined_external_symbols(nm_path, archive_path);

    for raw_symbol in raw_to_raw_renamed.keys() {
        if rewritten_symbols.contains(raw_symbol) {
            panic!("symbol rewrite failed, original symbol is still present: {raw_symbol}");
        }
    }

    for raw_renamed_symbol in raw_to_raw_renamed.values() {
        if !rewritten_symbols.contains(raw_renamed_symbol) {
            panic!("symbol rewrite failed, renamed symbol is missing: {raw_renamed_symbol}");
        }
    }
}

// C 識別子かどうかを判定する
fn is_c_identifier(symbol: &str) -> bool {
    if symbol.is_empty() {
        return false;
    }

    let mut chars = symbol.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }

    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
