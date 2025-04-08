use std::path::PathBuf;

// 依存ライブラリの名前
const LIB_NAME: &str = "fdk-aac";

// 環境変数で fdk-aac のインクルードパスを指定する場合のキー
// デフォルト: /usr/include/fdk-aac/
const ENV_FDK_AAC_INCLUDE_DIR: &str = "FDK_AAC_INCLUDE_DIR";

fn main() {
    // Cargo.toml か build.rs が更新されたら、依存ライブラリを再ビルドする
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rerun-if-changed=build.rs");

    // 各種変数やビルドディレクトリのセットアップ
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("infallible"));
    let out_build_dir = out_dir.join("build/");
    let output_bindings_path = out_dir.join("bindings.rs");
    let _ = std::fs::remove_dir_all(&out_build_dir);
    std::fs::create_dir(&out_build_dir).expect("failed to create build directory");

    if std::env::var("DOCS_RS").is_ok() {
        // Docs.rs 向けのビルドではシステムの FDK-AAC が参照できないので build.rs の処理はスキップして、
        // 代わりに、ドキュメント生成時に最低限必要な定義だけをダミーで出力している。
        //
        // See also: https://docs.rs/about/builds
        std::fs::write(
            output_bindings_path,
            concat!("pub struct AACENC_ERROR;", "pub struct HANDLE_AACENCODER;",),
        )
        .expect("write file error");
        return;
    }

    // バインディングを生成する
    let include_dir = PathBuf::from(
        std::env::var(ENV_FDK_AAC_INCLUDE_DIR)
            .ok()
            .unwrap_or_else(|| "/usr/include/fdk-aac/".to_owned()),
    );

    bindgen::Builder::default()
        .header(include_dir.join("aacenc_lib.h").display().to_string())
        .generate()
        .expect("failed to generate bindings")
        .write_to_file(output_bindings_path)
        .expect("failed to write bindings");

    println!("cargo::rustc-link-lib=dylib={LIB_NAME}");
}
