use std::{path::PathBuf, process::Command};

fn main() {
    // build.rs が更新されたら、依存ライブラリを再ビルドする
    println!("cargo::rerun-if-changed=build.rs");

    // 各種変数やビルドディレクトリのセットアップ
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("infallible"));
    let out_include_dir = out_dir.join("include/");
    let output_bindings_path = out_dir.join("bindings.rs");

    let _ = std::fs::remove_dir_all(&out_include_dir);
    std::fs::create_dir(&out_include_dir).expect("failed to create include directory");

    // Audio Toolbox の SDK のパスを取得する
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
        "CoreFoundation",
        "CoreAudioTypes",
        "CoreAudio",
        "AudioToolbox",
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
                .join("AudioToolbox/AudioToolbox.h")
                .display()
                .to_string(),
        )
        // Audio Toolbox 側のコメントが誤ってテスト対象と認識されてしまいエラーとなることがあるので、
        // コメントは生成しないようにしている。
        .generate_comments(false)
        .generate()
        .expect("failed to generate bindings")
        .write_to_file(output_bindings_path)
        .expect("failed to write bindings");

    println!("cargo::rustc-link-lib=framework=AudioToolbox");
}
