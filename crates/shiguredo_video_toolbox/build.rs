use std::{path::PathBuf, process::Command};

fn main() {
    // build.rs が更新されたら、依存ライブラリを再ビルドする
    println!("cargo::rerun-if-changed=build.rs");

    // 各種変数やビルドディレクトリのセットアップ
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("infallible"));
    let out_include_dir = out_dir.join("include/");
    let output_bindings_path = out_dir.join("bindings.rs");

    if std::env::var("DOCS_RS").is_ok() {
        // Docs.rs 向けのビルドでは Video Toolbox は参照できないので build.rs の処理はスキップして、
        // 代わりに、ドキュメント生成時に最低限必要な定義だけをダミーで出力している。
        //
        // See also: https://docs.rs/about/builds
        std::fs::write(
            output_bindings_path,
            concat!(
                "pub struct CFDictionaryRef;",
                "pub struct CFStringRef;",
                "pub struct __CVBuffer;",
                "pub struct CMTime;",
                "pub struct CVImageBufferRef;",
                "pub struct VTDecodeInfoFlags;",
                "pub struct VTDecompressionSessionRef;",
                "pub struct CMVideoFormatDescriptionRef;",
                "pub struct CMSampleBufferRef;",
                "pub struct VTEncodeInfoFlags;",
                "pub struct VTCompressionSessionRef;",
                "pub struct VTCompressionSessionCreate;",
            ),
        )
        .expect("write file error");
        return;
    }

    let _ = std::fs::remove_dir_all(&out_include_dir);
    std::fs::create_dir(&out_include_dir).expect("failed to create include directory");

    // Video Toolbox の SDK のパスを取得する
    let output = Command::new("xcrun")
        .arg("--show-sdk-path")
        .output()
        .expect("failed to execute `xcrun` command");
    let sdk_dir = PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("invalid path")
            .trim(),
    );

    // bindgen が解釈可能な構成にヘッダファイルを配置し直す
    let frameworks = [
        "IOKit",
        "OpenGL",
        "CoreFoundation",
        "CoreMedia",
        "CoreGraphics",
        "CoreAudio",
        "CoreAudioTypes",
        "CoreVideo",
        "VideoToolbox",
    ];
    for framework in &frameworks {
        let framework_headers_dir = sdk_dir.join(format!(
            "System/Library/Frameworks/{framework}.framework/Versions/A/Headers/"
        ));
        std::os::unix::fs::symlink(framework_headers_dir, out_include_dir.join(framework))
            .expect("failed to create a symlink");
    }

    // バインディングを生成する
    bindgen::Builder::default()
        .clang_arg(format!("-I{}", out_include_dir.display()))
        .header(
            out_include_dir
                .join("VideoToolbox/VideoToolbox.h")
                .display()
                .to_string(),
        )
        // Video Toolbox 側のコメントが誤ってテスト対象と認識されてしまいエラーとなることがあるので、
        // コメントは生成しないようにしている。
        .generate_comments(false)
        .generate()
        .expect("failed to generate bindings")
        .write_to_file(output_bindings_path)
        .expect("failed to write bindings");

    println!("cargo::rustc-link-lib=framework=CoreFoundation");
    println!("cargo::rustc-link-lib=framework=CoreMedia");
    println!("cargo::rustc-link-lib=framework=CoreVideo");
    println!("cargo::rustc-link-lib=framework=VideoToolbox");
}
