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
    let nvcuvid_header = third_party_header_dir.join("nvcuvid.h");

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
    let mut builder = bindgen::Builder::default()
        .header(nvenc_header.display().to_string())
        .header(cuvid_header.display().to_string());

    // nvcuvid.h があれば追加
    if nvcuvid_header.exists() {
        builder = builder.header(nvcuvid_header.display().to_string());
    }

    let bindings = builder
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
        // Block GUID extern static declarations
        .blocklist_item("NV_ENC_CODEC_H264_GUID")
        .blocklist_item("NV_ENC_CODEC_HEVC_GUID")
        .blocklist_item("NV_ENC_CODEC_AV1_GUID")
        .blocklist_item("NV_ENC_CODEC_PROFILE_AUTOSELECT_GUID")
        .blocklist_item("NV_ENC_H264_PROFILE_BASELINE_GUID")
        .blocklist_item("NV_ENC_H264_PROFILE_MAIN_GUID")
        .blocklist_item("NV_ENC_H264_PROFILE_HIGH_GUID")
        .blocklist_item("NV_ENC_H264_PROFILE_HIGH_10_GUID")
        .blocklist_item("NV_ENC_H264_PROFILE_HIGH_422_GUID")
        .blocklist_item("NV_ENC_H264_PROFILE_HIGH_444_GUID")
        .blocklist_item("NV_ENC_H264_PROFILE_STEREO_GUID")
        .blocklist_item("NV_ENC_H264_PROFILE_PROGRESSIVE_HIGH_GUID")
        .blocklist_item("NV_ENC_H264_PROFILE_CONSTRAINED_HIGH_GUID")
        .blocklist_item("NV_ENC_HEVC_PROFILE_MAIN_GUID")
        .blocklist_item("NV_ENC_HEVC_PROFILE_MAIN10_GUID")
        .blocklist_item("NV_ENC_HEVC_PROFILE_FREXT_GUID")
        .blocklist_item("NV_ENC_AV1_PROFILE_MAIN_GUID")
        .blocklist_item("NV_ENC_PRESET_P1_GUID")
        .blocklist_item("NV_ENC_PRESET_P2_GUID")
        .blocklist_item("NV_ENC_PRESET_P3_GUID")
        .blocklist_item("NV_ENC_PRESET_P4_GUID")
        .blocklist_item("NV_ENC_PRESET_P5_GUID")
        .blocklist_item("NV_ENC_PRESET_P6_GUID")
        .blocklist_item("NV_ENC_PRESET_P7_GUID")
        // 関数ポインタの生成を有効化
        .generate_comments(false)
        .derive_debug(false)
        .derive_default(false)
        .generate()
        .expect("failed to generate bindings");

    // Add version constants that are defined as macros in the header
    let version_constants = r#"

// Version constants from nvEncodeAPI.h
// These are macros in C, so bindgen doesn't generate them automatically
const NVENCAPI_STRUCT_VERSION_BASE: u32 = 0x7 << 28;

pub const NV_ENCODE_API_FUNCTION_LIST_VER: u32 = NVENCAPI_VERSION | (2 << 16) | NVENCAPI_STRUCT_VERSION_BASE;
pub const NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER: u32 = NVENCAPI_VERSION | (1 << 16) | NVENCAPI_STRUCT_VERSION_BASE;
pub const NV_ENC_PRESET_CONFIG_VER: u32 = NVENCAPI_VERSION | (5 << 16) | NVENCAPI_STRUCT_VERSION_BASE | (1 << 31);
pub const NV_ENC_CONFIG_VER: u32 = NVENCAPI_VERSION | (9 << 16) | NVENCAPI_STRUCT_VERSION_BASE | (1 << 31);
pub const NV_ENC_INITIALIZE_PARAMS_VER: u32 = NVENCAPI_VERSION | (7 << 16) | NVENCAPI_STRUCT_VERSION_BASE | (1 << 31);
pub const NV_ENC_CREATE_BITSTREAM_BUFFER_VER: u32 = NVENCAPI_VERSION | (1 << 16) | NVENCAPI_STRUCT_VERSION_BASE;
pub const NV_ENC_PIC_PARAMS_VER: u32 = NVENCAPI_VERSION | (7 << 16) | NVENCAPI_STRUCT_VERSION_BASE | (1 << 31);
pub const NV_ENC_LOCK_BITSTREAM_VER: u32 = NVENCAPI_VERSION | (2 << 16) | NVENCAPI_STRUCT_VERSION_BASE | (1 << 31);
pub const NV_ENC_REGISTER_RESOURCE_VER: u32 = NVENCAPI_VERSION | (5 << 16) | NVENCAPI_STRUCT_VERSION_BASE;
pub const NV_ENC_MAP_INPUT_RESOURCE_VER: u32 = NVENCAPI_VERSION | (4 << 16) | NVENCAPI_STRUCT_VERSION_BASE;

// Picture flags
pub const NV_ENC_PIC_FLAG_EOS: u32 = 0x8;
"#;

    // Write bindings with version constants appended
    let mut bindings_content = bindings.to_string();
    bindings_content.push_str(version_constants);

    std::fs::write(&output_bindings_path, bindings_content).expect("failed to write bindings");

    // CUDA と NVENC/NVCUVID ライブラリのリンク設定
    println!("cargo::rustc-link-lib=dylib=cuda");
    println!("cargo::rustc-link-lib=dylib=nvcuvid");
    println!("cargo::rustc-link-lib=dylib=nvidia-encode");
}
