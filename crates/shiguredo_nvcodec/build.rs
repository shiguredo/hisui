use std::path::PathBuf;

fn main() {
    // Cargo.toml か build.rs が更新されたら、依存ライブラリを再ビルドする
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rerun-if-changed=build.rs");
    // third_party のヘッダファイルが更新されたら再ビルドする
    println!("cargo::rerun-if-changed=../../third_party/nvcodec/include/");

    // 各種変数やビルドディレクトリのセットアップ
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("infallible"));
    let output_bindings_path = out_dir.join("bindings.rs");

    // third_party にあるヘッダファイルのパス
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("infallible"));
    let third_party_header_dir = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("failed to get project root")
        .join("third_party/nvcodec/include");

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

    // third_party のヘッダファイルが存在することを確認
    if !third_party_header_dir.exists() {
        panic!(
            "Third party nvcodec headers not found at {:?}. Please ensure the headers are placed in third_party/nvcodec/include/",
            third_party_header_dir
        );
    }

    let nvenc_header = third_party_header_dir.join("nvEncodeAPI.h");
    let cuvid_header = third_party_header_dir.join("cuviddec.h");

    if !nvenc_header.exists() {
        panic!("nvEncodeAPI.h not found at {:?}", nvenc_header);
    }
    if !cuvid_header.exists() {
        panic!("cuviddec.h not found at {:?}", cuvid_header);
    }

    // CUDA include path を検出
    let cuda_include_paths = vec![
        "/usr/lib/cuda/include",
        "/usr/local/cuda/include",
        "/opt/cuda/include",
        "/usr/include/cuda",
    ];

    let cuda_include_path = cuda_include_paths
        .iter()
        .find(|&&path| PathBuf::from(path).join("cuda.h").exists())
        .expect("CUDA headers not found. Please install CUDA toolkit.");

    // バインディングを生成する
    bindgen::Builder::default()
        .header(nvenc_header.display().to_string())
        .header(cuvid_header.display().to_string())
        .header(
            PathBuf::from(cuda_include_path)
                .join("cuda.h")
                .display()
                .to_string(),
        )
        // CUDA include pathを追加
        .clang_arg(format!("-I{}", cuda_include_path))
        // third_party include pathも追加
        .clang_arg(format!("-I{}", third_party_header_dir.display()))
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
