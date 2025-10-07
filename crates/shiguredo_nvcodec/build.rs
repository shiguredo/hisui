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
                "pub struct NV_ENC_INPUT_PTR;",
                "pub struct NV_ENC_OUTPUT_PTR;",
                "pub struct cudaVideoCodec;",
                "pub struct CUVIDEOFORMAT;",
                "pub struct CUVIDPICPARAMS;",
                "pub struct CUVIDPARSERDISPINFO;",
                "pub struct CUdeviceptr;",
                "pub struct CUvideoparser;",
                "pub struct CUvideodecoder;",
                "pub struct CUcontext;",
                "pub struct CUvideoctxlock;",
                "pub struct GUID;",
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

// プロファイル GUID: NV_ENC_H264_PROFILE_MAIN_GUID
// {60B5C1D4-67FE-4790-94D5-C4726D7B6E6D}
pub const NV_ENC_H264_PROFILE_MAIN_GUID: GUID = GUID {
    Data1: 0x60b5c1d4,
    Data2: 0x67fe,
    Data3: 0x4790,
    Data4: [0x94, 0xd5, 0xc4, 0x72, 0x6d, 0x7b, 0x6e, 0x6d],
};

// プロファイル GUID: NV_ENC_HEVC_PROFILE_MAIN_GUID
// {B514C39A-B55B-40fa-878F-F1253B4DFDEC}
pub const NV_ENC_HEVC_PROFILE_MAIN_GUID: GUID = GUID {
    Data1: 0xb514c39a,
    Data2: 0xb55b,
    Data3: 0x40fa,
    Data4: [0x87, 0x8f, 0xf1, 0x25, 0x3b, 0x4d, 0xfd, 0xec],
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

#[derive(Debug)]
struct CustomCallbacks;

impl bindgen::callbacks::ParseCallbacks for CustomCallbacks {
    fn add_derives(&self, info: &bindgen::callbacks::DeriveInfo<'_>) -> Vec<String> {
        // "_GUID" に PartialEq と Eq を導出
        if info.name == "_GUID" {
            vec!["PartialEq".to_string(), "Eq".to_string()]
        } else {
            vec![]
        }
    }
}
