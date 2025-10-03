use std::path::PathBuf;

fn main() {
    // Cargo.toml か build.rs か third_party のヘッダファイルが更新されたら、バインディングファイルを再生成する
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=../../third_party/nvcodec/include/");

    // 各種変数やビルドディレクトリのセットアップ
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("infallible"));
    let output_bindings_path = out_dir.join("bindings.rs");
    let output_metadata_path = out_dir.join("metadata.rs");

    // 各種メタデータを書き込む
    let version = get_version();
    std::fs::write(
        output_metadata_path,
        format!("pub const BUILD_METADATA_VERSION: &str={:?};\n", version),
    )
    .expect("failed to write metadata file");

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
                "pub struct NV_ENC_BUFFER_FORMAT;",
                "pub struct NV_ENC_OUTPUT_PTR;",
                "pub struct CUVIDEOFORMAT;",
                "pub struct CUVIDPICPARAMS;",
                "pub struct CUVIDPARSERDISPINFO;",
                "pub struct CUdeviceptr;",
                "pub struct CUvideoparser;",
                "pub struct CUvideodecoder;",
                "pub struct CUcontext;",
                "pub struct CUvideoctxlock;",
                "pub struct NV_ENCODE_API_FUNCTION_LIST;",
                "pub struct NV_ENC_REGISTERED_PTR;",
                "pub struct NV_ENC_PIC_TYPE;",
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
    if !nvcuvid_header.exists() {
        panic!("nvcuvid.h not found at {:?}", nvcuvid_header);
    }

    // バインディングを生成する
    let bindings = bindgen::Builder::default()
        .header(nvenc_header.display().to_string())
        .header(cuvid_header.display().to_string())
        .header(nvcuvid_header.display().to_string())
        .generate_comments(false)
        .derive_debug(false)
        .derive_default(false)
        // GUID は bindgen で正しく生成されないため、ここではブラックリストに登録して、後で手動で定義する
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
        .generate()
        .expect("failed to generate bindings");

    // バージョン定数と GUID 定義を追加する
    let additional_definitions = r#"

// nvEncodeAPI.h のバージョン定数
// これらは C のマクロなので、bindgen は自動的に生成しない
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

// ピクチャーフラグ
pub const NV_ENC_PIC_FLAG_EOS: u32 = 0x8;

// crate で使用される NVENC GUID 定数
// これらの GUID はリンクの問題を避けるために extern static ではなく定数として定義されている。

// コーデック GUID: NV_ENC_CODEC_HEVC_GUID
// {790CDC88-4522-4d7b-9425-BDA9975F7603}
pub const NV_ENC_CODEC_HEVC_GUID: GUID = GUID {
    Data1: 0x790cdc88,
    Data2: 0x4522,
    Data3: 0x4d7b,
    Data4: [0x94, 0x25, 0xbd, 0xa9, 0x97, 0x5f, 0x76, 0x03],
};

// プリセット GUID: NV_ENC_PRESET_P4_GUID
// {90A7B826-DF06-4862-B9D2-CD6D73A08681}
pub const NV_ENC_PRESET_P4_GUID: GUID = GUID {
    Data1: 0x90a7b826,
    Data2: 0xdf06,
    Data3: 0x4862,
    Data4: [0xb9, 0xd2, 0xcd, 0x6d, 0x73, 0xa0, 0x86, 0x81],
};

// プロファイル GUID: NV_ENC_HEVC_PROFILE_MAIN_GUID
// {B514C39A-B55B-40fa-878F-F1253B4DFDEC}
pub const NV_ENC_HEVC_PROFILE_MAIN_GUID: GUID = GUID {
    Data1: 0xb514c39a,
    Data2: 0xb55b,
    Data3: 0x40fa,
    Data4: [0x87, 0x8f, 0xf1, 0x25, 0x3b, 0x4d, 0xfd, 0xec],
};
"#;

    // 追加の定義を付加してバインディングを書き込む
    std::fs::write(
        &output_bindings_path,
        format!("{bindings}\n{additional_definitions}"),
    )
    .expect("failed to write bindings");

    // CUDA と NVENC/NVCUVID ライブラリのリンク設定
    println!("cargo::rustc-link-lib=dylib=cuda");
    println!("cargo::rustc-link-lib=dylib=nvcuvid");
    println!("cargo::rustc-link-lib=dylib=nvidia-encode");
}

// Cargo.toml から依存ライブラリのバージョンを取得する
fn get_version() -> String {
    let cargo_toml: toml::Value =
        toml::from_str(include_str!("Cargo.toml")).expect("failed to parse Cargo.toml");
    if let Some(version) = cargo_toml
        .get("package")
        .and_then(|v| v.get("metadata"))
        .and_then(|v| v.get("external-dependencies"))
        .and_then(|v| v.get("nvcodec"))
        .and_then(|v| v.get("version"))
        .and_then(|s| s.as_str())
    {
        version.to_string()
    } else {
        panic!(
            "Cargo.toml does not contain a valid [package.metadata.external-dependencies.nvcodec] version"
        );
    }
}
