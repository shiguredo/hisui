use std::path::PathBuf;

const DEFAULT_CUDA_INCLUDE_PATH: &str = "/usr/local/cuda/include/";
const CUDA_INCLUDE_PATH_ENV_KEY: &str = "CUDA_INCLUDE_PATH";

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
                "pub struct NV_ENC_BUFFER_FORMAT;\n",
                "pub struct NV_ENC_INPUT_PTR;\n",
                "pub struct NV_ENC_OUTPUT_PTR;\n",
                "pub struct cudaVideoCodec;\n",
                "pub struct CUVIDEOFORMAT;\n",
                "pub struct CUVIDPICPARAMS;\n",
                "pub struct CUVIDPARSERDISPINFO;\n",
                "pub struct CUdeviceptr;\n",
                "pub struct CUvideoparser;\n",
                "pub struct CUvideodecoder;\n",
                "pub struct CUcontext;\n",
                "pub struct CUvideoctxlock;\n",
                "pub struct CUVIDPARSERPARAMS;\n",
                "pub struct CUVIDSOURCEDATAPACKET;\n",
                "pub struct CUVIDDECODECREATEINFO;\n",
                "pub struct CUVIDPROCPARAMS;\n",
                "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n",
                "pub struct GUID;\n",
                "pub struct NV_ENCODE_API_FUNCTION_LIST;\n",
                "pub struct NV_ENC_REGISTERED_PTR;\n",
                "pub struct NV_ENC_PIC_TYPE;\n",
                "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n",
                "pub struct NV_ENC_TUNING_INFO;\n",
                "pub struct NV_ENC_PARAMS_RC_MODE;\n",
                "pub const NV_ENC_PRESET_P1_GUID: GUID = GUID;\n",
                "pub const NV_ENC_PRESET_P2_GUID: GUID = GUID;\n",
                "pub const NV_ENC_PRESET_P3_GUID: GUID = GUID;\n",
                "pub const NV_ENC_PRESET_P4_GUID: GUID = GUID;\n",
                "pub const NV_ENC_PRESET_P5_GUID: GUID = GUID;\n",
                "pub const NV_ENC_PRESET_P6_GUID: GUID = GUID;\n",
                "pub const NV_ENC_PRESET_P7_GUID: GUID = GUID;\n",
                "pub const NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_HIGH_QUALITY: NV_ENC_TUNING_INFO = NV_ENC_TUNING_INFO;\n",
                "pub const NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_LOW_LATENCY: NV_ENC_TUNING_INFO = NV_ENC_TUNING_INFO;\n",
                "pub const NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_ULTRA_LOW_LATENCY: NV_ENC_TUNING_INFO = NV_ENC_TUNING_INFO;\n",
                "pub const NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_LOSSLESS: NV_ENC_TUNING_INFO = NV_ENC_TUNING_INFO;\n",
                "pub const NV_ENC_CODEC_PROFILE_AUTOSELECT_GUID: GUID = GUID;\n",
                "pub const NV_ENC_H264_PROFILE_BASELINE_GUID: GUID = GUID;\n",
                "pub const NV_ENC_H264_PROFILE_MAIN_GUID: GUID = GUID;\n",
                "pub const NV_ENC_H264_PROFILE_HIGH_GUID: GUID = GUID;\n",
                "pub const NV_ENC_H264_PROFILE_HIGH_10_GUID: GUID = GUID;\n",
                "pub const NV_ENC_H264_PROFILE_HIGH_422_GUID: GUID = GUID;\n",
                "pub const NV_ENC_H264_PROFILE_HIGH_444_GUID: GUID = GUID;\n",
                "pub const NV_ENC_H264_PROFILE_STEREO_GUID: GUID = GUID;\n",
                "pub const NV_ENC_H264_PROFILE_PROGRESSIVE_HIGH_GUID: GUID = GUID;\n",
                "pub const NV_ENC_H264_PROFILE_CONSTRAINED_HIGH_GUID: GUID = GUID;\n",
                "pub const NV_ENC_HEVC_PROFILE_MAIN_GUID: GUID = GUID;\n",
                "pub const NV_ENC_HEVC_PROFILE_MAIN10_GUID: GUID = GUID;\n",
                "pub const NV_ENC_HEVC_PROFILE_FREXT_GUID: GUID = GUID;\n",
                "pub const NV_ENC_AV1_PROFILE_MAIN_GUID: GUID = GUID;\n",
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

    // CUDA インクルードパスを取得
    let cuda_include_path = PathBuf::from(
        std::env::var(CUDA_INCLUDE_PATH_ENV_KEY)
            .unwrap_or_else(|_| DEFAULT_CUDA_INCLUDE_PATH.to_string()),
    );
    if !cuda_include_path.join("cuda.h").exists() {
        panic!(
            r#"cuda.h not found in the specified CUDA include directory.

Searched location: {}

To resolve this issue:
1. Ensure CUDA Toolkit is installed on your system
2. Set the environment variable {CUDA_INCLUDE_PATH_ENV_KEY} to point to your CUDA include directory
3. Alternatively, ensure cuda.h exists at the default location: {DEFAULT_CUDA_INCLUDE_PATH}
"#,
            cuda_include_path.join("cuda.h").display(),
        );
    }

    // バインディングを生成する
    let bindings = bindgen::Builder::default()
        .header(nvenc_header.display().to_string())
        .header(cuvid_header.display().to_string())
        .header(nvcuvid_header.display().to_string())
        .clang_arg(format!("-I{}", cuda_include_path.display()))
        .generate_comments(false)
        .derive_debug(false)
        .derive_default(false)
        .parse_callbacks(Box::new(CustomCallbacks))
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
pub const NV_ENC_SEQUENCE_PARAM_PAYLOAD_VER: u32 = NVENCAPI_VERSION | (1 << 16) | NVENCAPI_STRUCT_VERSION_BASE;

// ピクチャーフラグ
pub const NV_ENC_PIC_FLAG_EOS: u32 = 0x8;

// crate で使用される NVENC GUID 定数
// これらの GUID はリンクの問題を避けるために extern static ではなく定数として定義されている。

// コーデック GUID: NV_ENC_CODEC_H264_GUID
// {6BC82762-4E63-4ca4-AA85-1E50F321F6BF}
pub const NV_ENC_CODEC_H264_GUID: GUID = GUID {
    Data1: 0x6bc82762,
    Data2: 0x4e63,
    Data3: 0x4ca4,
    Data4: [0xaa, 0x85, 0x1e, 0x50, 0xf3, 0x21, 0xf6, 0xbf],
};

// コーデック GUID: NV_ENC_CODEC_HEVC_GUID
// {790CDC88-4522-4d7b-9425-BDA9975F7603}
pub const NV_ENC_CODEC_HEVC_GUID: GUID = GUID {
    Data1: 0x790cdc88,
    Data2: 0x4522,
    Data3: 0x4d7b,
    Data4: [0x94, 0x25, 0xbd, 0xa9, 0x97, 0x5f, 0x76, 0x03],
};

// コーデック GUID: NV_ENC_CODEC_AV1_GUID
// {0A352289-0AA7-4759-862D-5D15CD16D254}
pub const NV_ENC_CODEC_AV1_GUID: GUID = GUID {
    Data1: 0x0a352289,
    Data2: 0x0aa7,
    Data3: 0x4759,
    Data4: [0x86, 0x2d, 0x5d, 0x15, 0xcd, 0x16, 0xd2, 0x54],
};

// プロファイル GUID: NV_ENC_CODEC_PROFILE_AUTOSELECT_GUID
// {BFD6F8E7-233C-4341-8B3E-4818523803F4}
pub const NV_ENC_CODEC_PROFILE_AUTOSELECT_GUID: GUID = GUID {
    Data1: 0xbfd6f8e7,
    Data2: 0x233c,
    Data3: 0x4341,
    Data4: [0x8b, 0x3e, 0x48, 0x18, 0x52, 0x38, 0x03, 0xf4],
};

// プロファイル GUID: NV_ENC_H264_PROFILE_BASELINE_GUID
// {0727BCAA-78C4-4c83-8C2F-EF3DFF267C6A}
pub const NV_ENC_H264_PROFILE_BASELINE_GUID: GUID = GUID {
    Data1: 0x0727bcaa,
    Data2: 0x78c4,
    Data3: 0x4c83,
    Data4: [0x8c, 0x2f, 0xef, 0x3d, 0xff, 0x26, 0x7c, 0x6a],
};

// プロファイル GUID: NV_ENC_H264_PROFILE_MAIN_GUID
// {60B5C1D4-67FE-4790-94D5-C4726D7B6E6D}
pub const NV_ENC_H264_PROFILE_MAIN_GUID: GUID = GUID {
    Data1: 0x60b5c1d4,
    Data2: 0x67fe,
    Data3: 0x4790,
    Data4: [0x94, 0xd5, 0xc4, 0x72, 0x6d, 0x7b, 0x6e, 0x6d],
};

// プロファイル GUID: NV_ENC_H264_PROFILE_HIGH_GUID
// {E7CBC309-4F7A-4b89-AF2A-D537C92BE310}
pub const NV_ENC_H264_PROFILE_HIGH_GUID: GUID = GUID {
    Data1: 0xe7cbc309,
    Data2: 0x4f7a,
    Data3: 0x4b89,
    Data4: [0xaf, 0x2a, 0xd5, 0x37, 0xc9, 0x2b, 0xe3, 0x10],
};

// プロファイル GUID: NV_ENC_H264_PROFILE_HIGH_10_GUID
// {8F0C337E-186C-48E9-A69D-7A8334089758}
pub const NV_ENC_H264_PROFILE_HIGH_10_GUID: GUID = GUID {
    Data1: 0x8f0c337e,
    Data2: 0x186c,
    Data3: 0x48e9,
    Data4: [0xa6, 0x9d, 0x7a, 0x83, 0x34, 0x08, 0x97, 0x58],
};

// プロファイル GUID: NV_ENC_H264_PROFILE_HIGH_422_GUID
// {FF3242E9-613C-4295-A1E8-2A7FE94D8133}
pub const NV_ENC_H264_PROFILE_HIGH_422_GUID: GUID = GUID {
    Data1: 0xff3242e9,
    Data2: 0x613c,
    Data3: 0x4295,
    Data4: [0xa1, 0xe8, 0x2a, 0x7f, 0xe9, 0x4d, 0x81, 0x33],
};

// プロファイル GUID: NV_ENC_H264_PROFILE_HIGH_444_GUID
// {7AC663CB-A598-4960-B844-339B261A7D52}
pub const NV_ENC_H264_PROFILE_HIGH_444_GUID: GUID = GUID {
    Data1: 0x7ac663cb,
    Data2: 0xa598,
    Data3: 0x4960,
    Data4: [0xb8, 0x44, 0x33, 0x9b, 0x26, 0x1a, 0x7d, 0x52],
};

// プロファイル GUID: NV_ENC_H264_PROFILE_STEREO_GUID
// {40847BF5-33F7-4601-9084-E8FE3C1DB8B7}
pub const NV_ENC_H264_PROFILE_STEREO_GUID: GUID = GUID {
    Data1: 0x40847bf5,
    Data2: 0x33f7,
    Data3: 0x4601,
    Data4: [0x90, 0x84, 0xe8, 0xfe, 0x3c, 0x1d, 0xb8, 0xb7],
};

// プロファイル GUID: NV_ENC_H264_PROFILE_PROGRESSIVE_HIGH_GUID
// {B405AFAC-F32B-417B-89C4-9ABEED3E5978}
pub const NV_ENC_H264_PROFILE_PROGRESSIVE_HIGH_GUID: GUID = GUID {
    Data1: 0xb405afac,
    Data2: 0xf32b,
    Data3: 0x417b,
    Data4: [0x89, 0xc4, 0x9a, 0xbe, 0xed, 0x3e, 0x59, 0x78],
};

// プロファイル GUID: NV_ENC_H264_PROFILE_CONSTRAINED_HIGH_GUID
// {AEC1BD87-E85B-48f2-84C3-98BCA6285072}
pub const NV_ENC_H264_PROFILE_CONSTRAINED_HIGH_GUID: GUID = GUID {
    Data1: 0xaec1bd87,
    Data2: 0xe85b,
    Data3: 0x48f2,
    Data4: [0x84, 0xc3, 0x98, 0xbc, 0xa6, 0x28, 0x50, 0x72],
};

// プロファイル GUID: NV_ENC_HEVC_PROFILE_MAIN_GUID
// {B514C39A-B55B-40fa-878F-F1253B4DFDEC}
pub const NV_ENC_HEVC_PROFILE_MAIN_GUID: GUID = GUID {
    Data1: 0xb514c39a,
    Data2: 0xb55b,
    Data3: 0x40fa,
    Data4: [0x87, 0x8f, 0xf1, 0x25, 0x3b, 0x4d, 0xfd, 0xec],
};

// プロファイル GUID: NV_ENC_HEVC_PROFILE_MAIN10_GUID
// {fa4d2b6c-3a5b-411a-8018-0a3f5e3c9be5}
pub const NV_ENC_HEVC_PROFILE_MAIN10_GUID: GUID = GUID {
    Data1: 0xfa4d2b6c,
    Data2: 0x3a5b,
    Data3: 0x411a,
    Data4: [0x80, 0x18, 0x0a, 0x3f, 0x5e, 0x3c, 0x9b, 0xe5],
};

// プロファイル GUID: NV_ENC_HEVC_PROFILE_FREXT_GUID
// {51ec32b5-1b4c-453c-9cbd-b616bd621341}
pub const NV_ENC_HEVC_PROFILE_FREXT_GUID: GUID = GUID {
    Data1: 0x51ec32b5,
    Data2: 0x1b4c,
    Data3: 0x453c,
    Data4: [0x9c, 0xbd, 0xb6, 0x16, 0xbd, 0x62, 0x13, 0x41],
};

// プロファイル GUID: NV_ENC_AV1_PROFILE_MAIN_GUID
// {5f2a39f5-f14e-4f95-9a9e-b76d568fcf97}
pub const NV_ENC_AV1_PROFILE_MAIN_GUID: GUID = GUID {
    Data1: 0x5f2a39f5,
    Data2: 0xf14e,
    Data3: 0x4f95,
    Data4: [0x9a, 0x9e, 0xb7, 0x6d, 0x56, 0x8f, 0xcf, 0x97],
};

// プリセット GUID: NV_ENC_PRESET_P1_GUID
// {FC0A8D3E-45F8-4CF8-80C7-298871590EBF}
pub const NV_ENC_PRESET_P1_GUID: GUID = GUID {
    Data1: 0xfc0a8d3e,
    Data2: 0x45f8,
    Data3: 0x4cf8,
    Data4: [0x80, 0xc7, 0x29, 0x88, 0x71, 0x59, 0x0e, 0xbf],
};

// プリセット GUID: NV_ENC_PRESET_P2_GUID
// {F581CFB8-88D6-4381-93F0-DF13F9C27DAB}
pub const NV_ENC_PRESET_P2_GUID: GUID = GUID {
    Data1: 0xf581cfb8,
    Data2: 0x88d6,
    Data3: 0x4381,
    Data4: [0x93, 0xf0, 0xdf, 0x13, 0xf9, 0xc2, 0x7d, 0xab],
};

// プリセット GUID: NV_ENC_PRESET_P3_GUID
// {36850110-3A07-441F-94D5-3670631F91F6}
pub const NV_ENC_PRESET_P3_GUID: GUID = GUID {
    Data1: 0x36850110,
    Data2: 0x3a07,
    Data3: 0x441f,
    Data4: [0x94, 0xd5, 0x36, 0x70, 0x63, 0x1f, 0x91, 0xf6],
};

// プリセット GUID: NV_ENC_PRESET_P4_GUID
// {90A7B826-DF06-4862-B9D2-CD6D73A08681}
pub const NV_ENC_PRESET_P4_GUID: GUID = GUID {
    Data1: 0x90a7b826,
    Data2: 0xdf06,
    Data3: 0x4862,
    Data4: [0xb9, 0xd2, 0xcd, 0x6d, 0x73, 0xa0, 0x86, 0x81],
};

// プリセット GUID: NV_ENC_PRESET_P5_GUID
// {21C6E6B4-297A-4CBA-998F-B6CBDE72ADE3}
pub const NV_ENC_PRESET_P5_GUID: GUID = GUID {
    Data1: 0x21c6e6b4,
    Data2: 0x297a,
    Data3: 0x4cba,
    Data4: [0x99, 0x8f, 0xb6, 0xcb, 0xde, 0x72, 0xad, 0xe3],
};

// プリセット GUID: NV_ENC_PRESET_P6_GUID
// {8E75C279-6299-4AB6-8302-0B215A335CF5}
pub const NV_ENC_PRESET_P6_GUID: GUID = GUID {
    Data1: 0x8e75c279,
    Data2: 0x6299,
    Data3: 0x4ab6,
    Data4: [0x83, 0x02, 0x0b, 0x21, 0x5a, 0x33, 0x5c, 0xf5],
};

// プリセット GUID: NV_ENC_PRESET_P7_GUID
// {84848C12-6F71-4C13-931B-53E283F57974}
pub const NV_ENC_PRESET_P7_GUID: GUID = GUID {
    Data1: 0x84848c12,
    Data2: 0x6f71,
    Data3: 0x4c13,
    Data4: [0x93, 0x1b, 0x53, 0xe2, 0x83, 0xf5, 0x79, 0x74],
};
"#;

    // 追加の定義を付加してバインディングを書き込む
    std::fs::write(
        &output_bindings_path,
        format!("{bindings}\n{additional_definitions}"),
    )
    .expect("failed to write bindings");
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

#[derive(Debug)]
struct CustomCallbacks;

impl bindgen::callbacks::ParseCallbacks for CustomCallbacks {
    fn add_derives(&self, info: &bindgen::callbacks::DeriveInfo<'_>) -> Vec<String> {
        // "_GUID" に各種トレイトを導出
        if info.name == "_GUID" {
            vec![
                "Debug".to_string(),
                "PartialEq".to_string(),
                "Eq".to_string(),
            ]
        } else {
            vec![]
        }
    }
}
