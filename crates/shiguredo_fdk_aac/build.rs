use std::path::PathBuf;

// 依存ライブラリの名前
const LIB_NAME: &str = "fdk-aac";

// 環境変数で fdk-aac のインクルードパスを指定する場合のキー名
// デフォルト: /usr/include/fdk-aac/
//
// 基本的には未指定（デフォルト）で問題ないはずだけど、
// システムにインストールされている fdk-aac のパスが通常とは異なる場合などには
// この環境変数を指定する必要がある
const ENV_FDK_AAC_INCLUDE_DIR: &str = "FDK_AAC_INCLUDE_DIR";

// 開発者向けの環境変数
//
// これが指定されている場合には、システムのものではなく
// 指定されたパスに配置された fdk-aac のソースを使ってバインディングを生成するようになる
// (macOS で cargo publish を行う際などに指定が必要となる）
const ENV_FDK_AAC_SOURCE_DIR: &str = "FDK_AAC_SOURCE_DIR";

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
    if let Ok(source_dir) = std::env::var(ENV_FDK_AAC_SOURCE_DIR) {
        // [NOTE]
        // システムにインストールされたものではなく、
        // fdk-aac のソースを直接参照してバインディングを生成する場合には
        // ヘッダファイルの配置構成が異なっている
        let source_dir = PathBuf::from(source_dir);
        let libaacenc_include_dir = source_dir.join("libAACenc/include/");
        let libsys_include_dir = source_dir.join("libSYS/include/");
        bindgen::Builder::default()
            .clang_arg(format!("-I{}", libsys_include_dir.display()))
            .header(
                libaacenc_include_dir
                    .join("aacenc_lib.h")
                    .display()
                    .to_string(),
            )
            .generate()
            .expect("failed to generate bindings")
            .write_to_file(output_bindings_path)
            .expect("failed to write bindings");
    } else {
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
    }

    println!("cargo::rustc-link-lib=dylib={LIB_NAME}");
}
