use std::path::{Path, PathBuf};

use git2::Repository;

// 依存ライブラリの名前
const LIB_NAME: &str = "libyuv";
const LINK_NAME: &str = "yuv";

fn main() {
    // Cargo.toml か build.rs が更新されたら、依存ライブラリを再ビルドする
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rerun-if-changed=build.rs");

    // 各種変数やビルドディレクトリのセットアップ
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("infallible"));
    let out_build_dir = out_dir.join("build/");
    let src_dir = out_build_dir.join(LIB_NAME);
    let input_header_dir = src_dir.join("include/");
    let output_metadata_path = out_dir.join("metadata.rs");
    let output_bindings_path = out_dir.join("bindings.rs");
    let _ = std::fs::remove_dir_all(&out_build_dir);
    std::fs::create_dir(&out_build_dir).expect("failed to create build directory");

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
        // 代わりに、ドキュメント生成時に最低限必要な定義だけをダミーで出力している。
        // NOTE: 今のところ libyuv では Docs.rs 用に必要な型定義はないので、空ファイルでいい
        //
        // See also: https://docs.rs/about/builds
        std::fs::write(output_bindings_path, "").expect("write file error");
        return;
    }

    // 依存ライブラリのリポジトリを取得する
    git_clone_external_lib(&out_build_dir);

    // 依存ライブラリをビルドする（cmake クレートを使用）
    let dst = cmake::Config::new(&src_dir)
        .define("CMAKE_BUILD_TYPE", "Release")
        .define("BUILD_SHARED_LIBS", "OFF")
        .build();

    // バインディングを生成する
    bindgen::Builder::default()
        .clang_arg(format!("-I{}", input_header_dir.display()))
        .header(input_header_dir.join("libyuv.h").display().to_string())
        .generate()
        .expect("failed to generate bindings")
        .write_to_file(output_bindings_path)
        .expect("failed to write bindings");

    println!("cargo::rustc-link-search={}", dst.join("lib").display());
    println!("cargo::rustc-link-lib=static={LINK_NAME}");
}

// 外部ライブラリのリポジトリを git clone する（git2 クレートを使用）
fn git_clone_external_lib(build_dir: &Path) {
    let (git_url, version) = get_git_url_and_version();
    let repo_dir = build_dir.join(LIB_NAME);

    // リポジトリを clone する
    let repo = Repository::clone(&git_url, &repo_dir)
        .unwrap_or_else(|e| panic!("failed to clone {LIB_NAME} repository: {e}"));

    // 指定されたバージョン（コミットハッシュまたはタグ）に checkout する
    let (object, reference) = repo
        .revparse_ext(&version)
        .unwrap_or_else(|e| panic!("failed to find revision {version}: {e}"));

    repo.checkout_tree(&object, None)
        .unwrap_or_else(|e| panic!("failed to checkout tree: {e}"));

    match reference {
        Some(gref) => repo
            .set_head(gref.name().unwrap())
            .unwrap_or_else(|e| panic!("failed to set head: {e}")),
        None => repo
            .set_head_detached(object.id())
            .unwrap_or_else(|e| panic!("failed to set head detached: {e}")),
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
