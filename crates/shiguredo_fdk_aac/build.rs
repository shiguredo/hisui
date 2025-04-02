use std::{
    path::{Path, PathBuf},
    process::Command,
};

// 依存ライブラリの名前
const LIB_NAME: &str = "fdk-aac";

fn main() {
    // Cargo.toml か build.rs が更新されたら、依存ライブラリを再ビルドする
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rerun-if-changed=build.rs");

    // 各種変数やビルドディレクトリのセットアップ
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("infallible"));
    let out_build_dir = out_dir.join("build/");
    let src_dir = out_build_dir.join(LIB_NAME);
    let output_lib_dir = src_dir.join(".libs/");
    let output_bindings_path = out_dir.join("bindings.rs");
    let _ = std::fs::remove_dir_all(&out_build_dir);
    std::fs::create_dir(&out_build_dir).expect("failed to create build directory");

    // 依存ライブラリのリポジトリを取得する
    git_clone_external_lib(&out_build_dir);

    // 依存ライブラリをビルドする
    let success = Command::new("autoreconf")
        .arg("-i")
        .current_dir(&src_dir)
        .status()
        .is_ok_and(|status| status.success());
    if !success {
        panic!("[autoreconf] failed to build {LIB_NAME}");
    }

    let success = Command::new("./configure")
        .arg("--enable-static=yes")
        .arg("--enable-shared=no")
        .current_dir(&src_dir)
        .status()
        .is_ok_and(|status| status.success());
    if !success {
        panic!("[configure] failed to build {LIB_NAME}");
    }

    let success = Command::new("make")
        .current_dir(&src_dir)
        .status()
        .is_ok_and(|status| status.success());
    if !success {
        panic!("[make] failed to build {LIB_NAME}");
    }

    // バインディングを生成する
    bindgen::Builder::default()
        .clang_arg(format!("-I{}", src_dir.join("libSYS/include/").display()))
        .header(
            src_dir
                .join("libSYS/include/machine_type.h")
                .display()
                .to_string(),
        )
        .header(
            src_dir
                .join("libSYS/include/FDK_audio.h")
                .display()
                .to_string(),
        )
        .header(
            src_dir
                .join("libAACenc/include/aacenc_lib.h")
                .display()
                .to_string(),
        )
        .generate()
        .expect("failed to generate bindings")
        .write_to_file(output_bindings_path)
        .expect("failed to write bindings");

    println!("cargo::rustc-link-search={}", output_lib_dir.display());
    println!("cargo::rustc-link-lib=static={LIB_NAME}");
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
            "Cargo.toml does not contains a valid [package.metadata.external-dependencies.{}] table",
             LIB_NAME
        );
    }
}
