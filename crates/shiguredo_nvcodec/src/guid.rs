//! NVENC GUID constants
//!
//! These GUIDs are defined as constants instead of extern static to avoid linking issues.

use crate::sys::GUID;

// Codec GUIDs
// {6BC82762-4E63-4ca4-AA85-1E50F321F6BF}
pub const NV_ENC_CODEC_H264_GUID: GUID = GUID {
    Data1: 0x6bc82762,
    Data2: 0x4e63,
    Data3: 0x4ca4,
    Data4: [0xaa, 0x85, 0x1e, 0x50, 0xf3, 0x21, 0xf6, 0xbf],
};

// {790CDC88-4522-4d7b-9425-BDA9975F7603}
pub const NV_ENC_CODEC_HEVC_GUID: GUID = GUID {
    Data1: 0x790cdc88,
    Data2: 0x4522,
    Data3: 0x4d7b,
    Data4: [0x94, 0x25, 0xbd, 0xa9, 0x97, 0x5f, 0x76, 0x03],
};

// {0A352289-0AA7-4759-862D-5D15CD16D254}
pub const NV_ENC_CODEC_AV1_GUID: GUID = GUID {
    Data1: 0x0a352289,
    Data2: 0x0aa7,
    Data3: 0x4759,
    Data4: [0x86, 0x2d, 0x5d, 0x15, 0xcd, 0x16, 0xd2, 0x54],
};

// Profile GUIDs
// {BFD6F8E7-233C-4341-8B3E-4818523803F4}
pub const NV_ENC_CODEC_PROFILE_AUTOSELECT_GUID: GUID = GUID {
    Data1: 0xbfd6f8e7,
    Data2: 0x233c,
    Data3: 0x4341,
    Data4: [0x8b, 0x3e, 0x48, 0x18, 0x52, 0x38, 0x03, 0xf4],
};

// H.264 Profile GUIDs
// {0727BCAA-78C4-4c83-8C2F-EF3DFF267C6A}
pub const NV_ENC_H264_PROFILE_BASELINE_GUID: GUID = GUID {
    Data1: 0x0727bcaa,
    Data2: 0x78c4,
    Data3: 0x4c83,
    Data4: [0x8c, 0x2f, 0xef, 0x3d, 0xff, 0x26, 0x7c, 0x6a],
};

// {60B5C1D4-67FE-4790-94D5-C4726D7B6E6D}
pub const NV_ENC_H264_PROFILE_MAIN_GUID: GUID = GUID {
    Data1: 0x60b5c1d4,
    Data2: 0x67fe,
    Data3: 0x4790,
    Data4: [0x94, 0xd5, 0xc4, 0x72, 0x6d, 0x7b, 0x6e, 0x6d],
};

// {E7CBC309-4F7A-4b89-AF2A-D537C92BE310}
pub const NV_ENC_H264_PROFILE_HIGH_GUID: GUID = GUID {
    Data1: 0xe7cbc309,
    Data2: 0x4f7a,
    Data3: 0x4b89,
    Data4: [0xaf, 0x2a, 0xd5, 0x37, 0xc9, 0x2b, 0xe3, 0x10],
};

// {B514C39A-B55B-40fa-878F-F1253B4DFDEC}
pub const NV_ENC_HEVC_PROFILE_MAIN_GUID: GUID = GUID {
    Data1: 0xb514c39a,
    Data2: 0xb55b,
    Data3: 0x40fa,
    Data4: [0x87, 0x8f, 0xf1, 0x25, 0x3b, 0x4d, 0xfd, 0xec],
};

// {fa4d2b6c-3a5b-411a-8018-0a3f5e3c9be5}
pub const NV_ENC_HEVC_PROFILE_MAIN10_GUID: GUID = GUID {
    Data1: 0xfa4d2b6c,
    Data2: 0x3a5b,
    Data3: 0x411a,
    Data4: [0x80, 0x18, 0x0a, 0x3f, 0x5e, 0x3c, 0x9b, 0xe5],
};

// Preset GUIDs
// {FC0A8D3E-45F8-4CF8-80C7-298871590EBF}
pub const NV_ENC_PRESET_P1_GUID: GUID = GUID {
    Data1: 0xfc0a8d3e,
    Data2: 0x45f8,
    Data3: 0x4cf8,
    Data4: [0x80, 0xc7, 0x29, 0x88, 0x71, 0x59, 0x0e, 0xbf],
};

// {F581CFB8-88D6-4381-93F0-DF13F9C27DAB}
pub const NV_ENC_PRESET_P2_GUID: GUID = GUID {
    Data1: 0xf581cfb8,
    Data2: 0x88d6,
    Data3: 0x4381,
    Data4: [0x93, 0xf0, 0xdf, 0x13, 0xf9, 0xc2, 0x7d, 0xab],
};

// {36850110-3A07-441F-94D5-3670631F91F6}
pub const NV_ENC_PRESET_P3_GUID: GUID = GUID {
    Data1: 0x36850110,
    Data2: 0x3a07,
    Data3: 0x441f,
    Data4: [0x94, 0xd5, 0x36, 0x70, 0x63, 0x1f, 0x91, 0xf6],
};

// {90A7B826-DF06-4862-B9D2-CD6D73A08681}
pub const NV_ENC_PRESET_P4_GUID: GUID = GUID {
    Data1: 0x90a7b826,
    Data2: 0xdf06,
    Data3: 0x4862,
    Data4: [0xb9, 0xd2, 0xcd, 0x6d, 0x73, 0xa0, 0x86, 0x81],
};

// {21C6E6B4-297A-4CBA-998F-B6CBDE72ADE3}
pub const NV_ENC_PRESET_P5_GUID: GUID = GUID {
    Data1: 0x21c6e6b4,
    Data2: 0x297a,
    Data3: 0x4cba,
    Data4: [0x99, 0x8f, 0xb6, 0xcb, 0xde, 0x72, 0xad, 0xe3],
};

// {8E75C279-6299-4AB6-8302-0B215A335CF5}
pub const NV_ENC_PRESET_P6_GUID: GUID = GUID {
    Data1: 0x8e75c279,
    Data2: 0x6299,
    Data3: 0x4ab6,
    Data4: [0x83, 0x02, 0x0b, 0x21, 0x5a, 0x33, 0x5c, 0xf5],
};

// {84848C12-6F71-4C13-931B-53E283F57974}
pub const NV_ENC_PRESET_P7_GUID: GUID = GUID {
    Data1: 0x84848c12,
    Data2: 0x6f71,
    Data3: 0x4c13,
    Data4: [0x93, 0x1b, 0x53, 0xe2, 0x83, 0xf5, 0x79, 0x74],
};
