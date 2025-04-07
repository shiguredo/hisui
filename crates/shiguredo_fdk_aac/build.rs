use std::path::PathBuf;

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

    // バインディングを生成する
    bindgen::Builder::default()
        // TODO: .clang_arg(format!("-I{}", src_dir.join("libSYS/include/").display()))
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
    println!("cargo::rustc-link-lib={LIB_NAME}");
}
