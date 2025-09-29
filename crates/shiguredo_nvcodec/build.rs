use std::{
    path::{Path, PathBuf},
    process::Command,
};

// 依存ライブラリの名前
const LIB_NAME: &str = "nvcodec";

fn main() {
    // Cargo.toml か build.rs が更新されたら、依存ライブラリを再ビルドする
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rerun-if-changed=build.rs");

    // 各種変数やビルドディレクトリのセットアップ
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("infallible"));
    let out_build_dir = out_dir.join("build/");
    let src_dir = out_build_dir.join(LIB_NAME);
    let input_header_dir = src_dir.join("Interface/");
    let output_metadata_path = out_dir.join("metadata.rs");
    let output_bindings_path = out_dir.join("bindings.rs");
    let _ = std::fs::remove_dir_all(&out_build_dir);
    std::fs::create_dir(&out_build_dir).expect("failed to create build directory");

    // 各種メタデータを書き込む
    let (download_url, version) = get_download_url_and_version();
    std::fs::write(
        output_metadata_path,
        format!(
            concat!(
                "pub const BUILD_METADATA_URL: &str={:?};\n",
                "pub const BUILD_METADATA_VERSION: &str={:?};\n",
            ),
            download_url, version
        ),
    )
    .expect("failed to write metadata file");

    if std::env::var("DOCS_RS").is_ok() {
        // Docs.rs 向けのビルドでは外部ファイルのダウンロードができないので build.rs の処理はスキップして、
        // 代わりに、ドキュメント生成時に最低限必要な定義だけをダミーで出力している。
        //
        // See also: https://docs.rs/about/builds
        std::fs::write(
            output_bindings_path,
            concat!(
                "pub struct NVENCAPI_MAJOR_VERSION;",
                "pub struct NVENCAPI_MINOR_VERSION;",
                "pub struct NV_ENC_INITIALIZE_PARAMS;",
                "pub struct NV_ENC_CONFIG;",
                "pub struct NV_ENC_BUFFER_FORMAT;",
                "pub struct NV_ENC_CODEC_TYPE;",
                "pub struct NVENCSTATUS;",
                "pub struct NV_ENC_INPUT_PTR;",
                "pub struct NV_ENC_OUTPUT_PTR;",
                "pub struct CUVIDSOURCEDATAPACKET;",
                "pub struct CUVIDEOFORMAT;",
                "pub struct CUVIDDECODECAPS;",
                "pub struct CUVIDDECODECREATEINFO;",
                "pub struct CUVIDPICPARAMS;",
                "pub struct CUVIDPARSERDISPINFO;",
                "pub struct CUvideoparser;",
                "pub struct CUvideodecoder;",
                "pub struct CUcontext;",
                "pub struct CUdevice;",
                "pub struct CUresult;",
            ),
        )
        .expect("write file error");
        return;
    }

    // 依存ライブラリのアーカイブを取得する
    download_and_extract_nvcodec_sdk(&out_build_dir, &version);

    // バインディングを生成する
    bindgen::Builder::default()
        .header(input_header_dir.join("nvEncodeAPI.h").display().to_string())
        .header(input_header_dir.join("cuviddec.h").display().to_string())
        .header(input_header_dir.join("cuda.h").display().to_string())
        // CUDA のバージョン定義を追加
        .clang_arg("-DCUDA_VERSION=13000")
        // 不要な警告を抑制
        .clang_arg("-Wno-everything")
        // 関数ポインタの生成を有効化
        .generate_comments(false)
        .derive_debug(false)
        .derive_default(false)
        .generate()
        .expect("failed to generate bindings")
        .write_to_file(output_bindings_path)
        .expect("failed to write bindings");

    // CUDA と NVENC ライブラリのリンク設定
    println!("cargo::rustc-link-lib=dylib=cuda");
    println!("cargo::rustc-link-lib=dylib=nvencodeapi");
    println!("cargo::rustc-link-lib=dylib=nvcuvid");
}

// NVIDIA Video Codec SDK のアーカイブをダウンロードして展開する
fn download_and_extract_nvcodec_sdk(build_dir: &Path, version: &str) {
    let (download_url, _) = get_download_url_and_version();
    let archive_name = format!("video_codec_interface_{}.zip", version);
    let archive_path = build_dir.join(&archive_name);

    // curl でアーカイブをダウンロード
    let success = Command::new("curl")
        .arg("-L") // リダイレクトに従う
        .arg("-o")
        .arg(&archive_path)
        .arg(&download_url)
        .current_dir(build_dir)
        .status()
        .is_ok_and(|status| status.success());
    if !success {
        panic!("failed to download {LIB_NAME} SDK archive");
    }

    // ZIP アーカイブを展開
    let success = Command::new("unzip")
        .arg("-q") // 静かに実行
        .arg(&archive_path)
        .arg("-d")
        .arg(build_dir)
        .current_dir(build_dir)
        .status()
        .is_ok_and(|status| status.success());
    if !success {
        panic!("failed to extract {LIB_NAME} SDK archive");
    }

    // アーカイブファイルを削除
    let _ = std::fs::remove_file(&archive_path);

    // 展開されたディレクトリを nvcodec にリネーム
    let extracted_dir = build_dir.join(format!("video_codec_interface_{}", version));
    if extracted_dir.exists() {
        let target_dir = build_dir.join(LIB_NAME);
        std::fs::rename(extracted_dir, target_dir).expect("failed to rename extracted directory");
    }
}

// Cargo.toml から NVIDIA Video Codec SDK のダウンロード URL とバージョンを取得する
fn get_download_url_and_version() -> (String, String) {
    let cargo_toml: toml::Value =
        toml::from_str(include_str!("Cargo.toml")).expect("failed to parse Cargo.toml");
    if let Some((Some(url), Some(version))) = cargo_toml
        .get("package")
        .and_then(|v| v.get("metadata"))
        .and_then(|v| v.get("external-dependencies"))
        .and_then(|v| v.get(LIB_NAME))
        .map(|v| {
            (
                v.get("url").and_then(|s| s.as_str()),
                v.get("version").and_then(|s| s.as_str()),
            )
        })
    {
        (url.to_string(), version.to_string())
    } else {
        panic!(
            "Cargo.toml does not contain a valid [package.metadata.external-dependencies.{LIB_NAME}] table"
        );
    }
}
