use std::{
    path::{Path, PathBuf},
    process::Command,
};

use cmake::Config;
use symbol_rewriter::rewrite_symbols;

#[path = "build/symbol_rewriter.rs"]
mod symbol_rewriter;

// 依存ライブラリの名前
const LIB_NAME: &str = "opus";
const LINK_NAME: &str = "opus";
const SHIGUREDO_OPUS_SYMBOL_PREFIX: &str = "shiguredo_opus_";

fn main() {
    // Cargo.toml か build.rs が更新されたら、依存ライブラリを再ビルドする
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=build/symbol_rewriter.rs");

    // 各種変数やビルドディレクトリのセットアップ
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("infallible"));
    let out_source_dir = out_dir.join("source/");
    let src_dir = out_source_dir.join(LIB_NAME);
    let output_metadata_path = out_dir.join("metadata.rs");
    let output_bindings_path = out_dir.join("bindings.rs");

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
    let callbacks = rewrite_symbols(
        &static_library_path,
        |symbol_name| Some(format!("{SHIGUREDO_OPUS_SYMBOL_PREFIX}{symbol_name}")),
        &out_dir,
        target_is_macos(),
    );

    // バインディングを生成する
    bindgen::Builder::default()
        .header(input_header_path.to_str().expect("invalid header path"))
        .parse_callbacks(callbacks)
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

// 対象プラットフォームが macOS かどうかを判定する
fn target_is_macos() -> bool {
    std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "macos")
}

