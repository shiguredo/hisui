/// HLS / DASH マニフェストに記載するコーデック文字列。
///
/// HLS のマスタープレイリスト（`CODECS` 属性）や DASH の MPD（`codecs` 属性）で使われる。
/// エンコーダーの設定に合わせて coordinator が構築し、各ライターに渡す。
#[derive(Debug, Clone)]
pub struct CodecString {
    /// ビデオコーデック文字列（例: "avc1.42e01f"）
    pub video: String,
    /// オーディオコーデック文字列（例: "mp4a.40.2"）
    pub audio: String,
}

impl CodecString {
    /// ビデオとオーディオの SampleEntry から正確な codec string を生成する。
    ///
    /// エンコーダーが実際に出力した SampleEntry を情報源とするため、
    /// MPD や HLS マニフェストに記載する codecs 属性はこのメソッドで生成すべき。
    /// いずれかの SampleEntry が非対応の場合は `None` を返す。
    pub fn from_sample_entries(
        video: &shiguredo_mp4::boxes::SampleEntry,
        audio: &shiguredo_mp4::boxes::SampleEntry,
    ) -> Option<Self> {
        let video_str = video_codec_string_from_sample_entry(video)?;
        let audio_str = audio_codec_string_from_sample_entry(audio)?;
        Some(Self {
            video: video_str,
            audio: audio_str,
        })
    }

    /// [`CodecName`] のペアから代表的な codec string を生成する。
    ///
    /// 実際のエンコーダー出力とはプロファイルやレベルが異なる可能性がある。
    /// MPD 等のマニフェスト本番用途では使わないこと。
    /// HLS のように codec が固定で SampleEntry を待てない場合のフォールバック用。
    pub fn from_codec_pair(video: crate::types::CodecName, audio: crate::types::CodecName) -> Self {
        use crate::types::CodecName;

        let video_str = match video {
            // H.264 Baseline Profile Level 3.1
            CodecName::H264 => "avc1.42e01f".to_owned(),
            // H.265 Main Profile, Main Tier, Level 3.1
            CodecName::H265 => "hev1.1.6.L93.B0".to_owned(),
            // AV1 Main Profile, Level 3.1, Main Tier, 8-bit
            CodecName::Av1 => "av01.0.05M.08".to_owned(),
            // VP9 Profile 0, Level 3.1, 8-bit
            CodecName::Vp9 => "vp09.00.31.08".to_owned(),
            // VP8 にはプロファイル/レベルのシグナリングがない
            CodecName::Vp8 => "vp8".to_owned(),
            CodecName::Aac | CodecName::Opus => {
                panic!("audio codec {:?} was passed as video codec", video)
            }
        };

        let audio_str = match audio {
            // AAC-LC (Audio Object Type 2)
            CodecName::Aac => "mp4a.40.2".to_owned(),
            CodecName::Opus => "opus".to_owned(),
            CodecName::H264
            | CodecName::H265
            | CodecName::Av1
            | CodecName::Vp8
            | CodecName::Vp9 => {
                panic!("video codec {:?} was passed as audio codec", audio)
            }
        };

        Self {
            video: video_str,
            audio: audio_str,
        }
    }

    /// "video_codec,audio_codec" 形式の結合文字列を返す。
    /// HLS の CODECS 属性や DASH の codecs 属性にそのまま使える。
    pub fn as_combined(&self) -> String {
        format!("{},{}", self.video, self.audio)
    }
}

/// SampleEntry からビデオコーデック文字列を生成する。
///
/// エンコーダーが出力した実際の SampleEntry からプロファイル・レベル等を読み取り、
/// MPD や HLS マニフェストに記載する正確な codec string を返す。
/// 対応していない SampleEntry の場合は `None` を返す。
pub fn video_codec_string_from_sample_entry(
    entry: &shiguredo_mp4::boxes::SampleEntry,
) -> Option<String> {
    use shiguredo_mp4::boxes::SampleEntry;
    match entry {
        SampleEntry::Avc1(b) => {
            let avcc = &b.avcc_box;
            // avc1.PPCCLL (PP=profile_idc, CC=constraint_set_flags, LL=level_idc)
            Some(format!(
                "avc1.{:02x}{:02x}{:02x}",
                avcc.avc_profile_indication, avcc.profile_compatibility, avcc.avc_level_indication,
            ))
        }
        SampleEntry::Hvc1(b) => Some(build_hevc_codec_string("hvc1", &b.hvcc_box)),
        SampleEntry::Hev1(b) => Some(build_hevc_codec_string("hev1", &b.hvcc_box)),
        SampleEntry::Av01(b) => {
            let av1c = &b.av1c_box;
            let profile = av1c.seq_profile.get();
            let level_idx = av1c.seq_level_idx_0.get();
            let tier = if av1c.seq_tier_0.get() == 0 { 'M' } else { 'H' };
            let bit_depth = if av1c.high_bitdepth.get() == 0 {
                8
            } else if av1c.twelve_bit.get() == 1 {
                12
            } else {
                10
            };
            // av01.P.LLT.DD
            Some(format!(
                "av01.{profile}.{level_idx:02}{tier}.{bit_depth:02}"
            ))
        }
        SampleEntry::Vp09(b) => {
            let vpcc = &b.vpcc_box;
            // vp09.PP.LL.DD
            Some(format!(
                "vp09.{:02}.{:02}.{:02}",
                vpcc.profile,
                vpcc.level,
                vpcc.bit_depth.get(),
            ))
        }
        SampleEntry::Vp08(_) => Some("vp8".to_owned()),
        _ => None,
    }
}

/// SampleEntry からオーディオコーデック文字列を生成する。
///
/// エンコーダーが出力した実際の SampleEntry から AudioSpecificConfig 等を読み取り、
/// MPD や HLS マニフェストに記載する正確な codec string を返す。
/// 対応していない SampleEntry の場合は `None` を返す。
pub fn audio_codec_string_from_sample_entry(
    entry: &shiguredo_mp4::boxes::SampleEntry,
) -> Option<String> {
    use shiguredo_mp4::boxes::SampleEntry;
    match entry {
        SampleEntry::Mp4a(b) => {
            // AudioSpecificConfig の先頭 5 bit が audio_object_type
            let aot = b
                .esds_box
                .es
                .dec_config_descr
                .dec_specific_info
                .as_ref()
                .and_then(|info| {
                    let payload = &info.payload;
                    if payload.is_empty() {
                        return None;
                    }
                    // 上位 5 bit が audio_object_type
                    let aot = payload[0] >> 3;
                    // audio_object_type == 31 の場合は拡張形式（5 bit + 6 bit）
                    if aot == 31 {
                        if payload.len() < 2 {
                            return None;
                        }
                        let ext = ((payload[0] & 0x07) << 3) | (payload[1] >> 5);
                        Some(u16::from(ext) + 32)
                    } else {
                        Some(u16::from(aot))
                    }
                })
                // AudioSpecificConfig が無い場合は AAC-LC を仮定
                .unwrap_or(2);
            Some(format!("mp4a.40.{aot}"))
        }
        SampleEntry::Opus(_) => Some("opus".to_owned()),
        _ => None,
    }
}

/// HEVC (H.265) の codec string を HvccBox から生成する。
///
/// RFC 6381 Section 3.3 に基づく形式:
/// `{prefix}.{profile}.{compat_flags_hex}.{tier}{level}.{constraints_hex}`
fn build_hevc_codec_string(prefix: &str, hvcc: &shiguredo_mp4::boxes::HvccBox) -> String {
    let profile_space = match hvcc.general_profile_space.get() {
        1 => "A",
        2 => "B",
        3 => "C",
        _ => "",
    };
    let profile_idc = hvcc.general_profile_idc.get();
    let tier = if hvcc.general_tier_flag.get() == 0 {
        'L'
    } else {
        'H'
    };
    let level = hvcc.general_level_idc;

    // general_profile_compatibility_flags を逆順ビットの 16 進数で表現する
    let compat = hvcc.general_profile_compatibility_flags.reverse_bits();
    let compat_hex = format!("{compat:X}");

    // general_constraint_indicator_flags (48 bit) を末尾のゼロバイトを除いた 16 進数で表現する
    let constraint_bytes = hvcc.general_constraint_indicator_flags.get().to_be_bytes();
    // 下位 6 バイト（[2..8]）が 48 bit の constraint flags
    let constraint_slice = &constraint_bytes[2..8];
    // 末尾のゼロバイトを除去する
    let last_nonzero = constraint_slice
        .iter()
        .rposition(|&b| b != 0)
        .map(|i| i + 1)
        .unwrap_or(1); // 全部ゼロでも最低 1 バイトは出力する
    let constraint_hex = constraint_slice[..last_nonzero]
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(".");

    format!("{prefix}.{profile_space}{profile_idc}.{compat_hex}.{tier}{level}.{constraint_hex}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CodecName;

    #[test]
    fn from_codec_pair_h264_aac() {
        let cs = CodecString::from_codec_pair(CodecName::H264, CodecName::Aac);
        assert_eq!(cs.video, "avc1.42e01f");
        assert_eq!(cs.audio, "mp4a.40.2");
        assert_eq!(cs.as_combined(), "avc1.42e01f,mp4a.40.2");
    }

    #[test]
    fn from_codec_pair_h265_aac() {
        let cs = CodecString::from_codec_pair(CodecName::H265, CodecName::Aac);
        assert_eq!(cs.video, "hev1.1.6.L93.B0");
        assert_eq!(cs.audio, "mp4a.40.2");
    }

    #[test]
    fn from_codec_pair_av1_opus() {
        let cs = CodecString::from_codec_pair(CodecName::Av1, CodecName::Opus);
        assert_eq!(cs.video, "av01.0.05M.08");
        assert_eq!(cs.audio, "opus");
    }

    #[test]
    fn from_codec_pair_vp9_opus() {
        let cs = CodecString::from_codec_pair(CodecName::Vp9, CodecName::Opus);
        assert_eq!(cs.video, "vp09.00.31.08");
        assert_eq!(cs.audio, "opus");
    }

    #[test]
    fn from_codec_pair_vp8_aac() {
        let cs = CodecString::from_codec_pair(CodecName::Vp8, CodecName::Aac);
        assert_eq!(cs.video, "vp8");
        assert_eq!(cs.audio, "mp4a.40.2");
    }

    #[test]
    #[should_panic(expected = "audio codec")]
    fn from_codec_pair_panics_on_audio_as_video() {
        CodecString::from_codec_pair(CodecName::Aac, CodecName::Aac);
    }

    #[test]
    #[should_panic(expected = "video codec")]
    fn from_codec_pair_panics_on_video_as_audio() {
        CodecString::from_codec_pair(CodecName::H264, CodecName::H264);
    }

    /// テスト用のデフォルト VisualSampleEntryFields を作る
    fn test_visual_fields() -> shiguredo_mp4::boxes::VisualSampleEntryFields {
        shiguredo_mp4::boxes::VisualSampleEntryFields {
            data_reference_index: std::num::NonZeroU16::MIN,
            width: 1920,
            height: 1080,
            horizresolution: shiguredo_mp4::FixedPointNumber {
                integer: 72,
                fraction: 0,
            },
            vertresolution: shiguredo_mp4::FixedPointNumber {
                integer: 72,
                fraction: 0,
            },
            frame_count: 1,
            compressorname: [0; 32],
            depth: 24,
        }
    }

    /// テスト用のデフォルト AudioSampleEntryFields を作る
    fn test_audio_fields() -> shiguredo_mp4::boxes::AudioSampleEntryFields {
        shiguredo_mp4::boxes::AudioSampleEntryFields {
            data_reference_index: std::num::NonZeroU16::MIN,
            channelcount: 2,
            samplesize: 16,
            samplerate: shiguredo_mp4::FixedPointNumber {
                integer: 48000,
                fraction: 0,
            },
        }
    }

    #[test]
    fn video_codec_string_from_avc1_sample_entry() {
        use shiguredo_mp4::Uint;
        use shiguredo_mp4::boxes::*;

        let entry = SampleEntry::Avc1(Avc1Box {
            visual: test_visual_fields(),
            avcc_box: AvccBox {
                // High Profile (100), constraint set flags 0x00, Level 4.0 (40)
                avc_profile_indication: 100,
                profile_compatibility: 0x00,
                avc_level_indication: 40,
                length_size_minus_one: Uint::new(3),
                sps_list: vec![],
                pps_list: vec![],
                chroma_format: Some(Uint::new(1)),
                bit_depth_luma_minus8: Some(Uint::new(0)),
                bit_depth_chroma_minus8: Some(Uint::new(0)),
                sps_ext_list: vec![],
            },
            unknown_boxes: vec![],
        });

        assert_eq!(
            video_codec_string_from_sample_entry(&entry),
            Some("avc1.640028".to_owned())
        );
    }

    #[test]
    fn audio_codec_string_from_mp4a_aac_lc() {
        use shiguredo_mp4::boxes::*;

        let entry = SampleEntry::Mp4a(Mp4aBox {
            audio: test_audio_fields(),
            esds_box: EsdsBox {
                es: shiguredo_mp4::descriptors::EsDescriptor {
                    es_id: 1,
                    stream_priority: shiguredo_mp4::Uint::new(0),
                    depends_on_es_id: None,
                    url_string: None,
                    ocr_es_id: None,
                    dec_config_descr: shiguredo_mp4::descriptors::DecoderConfigDescriptor {
                        object_type_indication: 0x40,
                        stream_type: shiguredo_mp4::Uint::new(0x05),
                        up_stream: shiguredo_mp4::Uint::new(0),
                        buffer_size_db: shiguredo_mp4::Uint::new(0),
                        max_bitrate: 128000,
                        avg_bitrate: 128000,
                        dec_specific_info: Some(shiguredo_mp4::descriptors::DecoderSpecificInfo {
                            // AAC-LC (type 2), 48kHz, stereo: 0b00010_0011_0010_000 = 0x11 0x90
                            payload: vec![0x11, 0x90],
                        }),
                    },
                    sl_config_descr: shiguredo_mp4::descriptors::SlConfigDescriptor,
                },
            },
            unknown_boxes: vec![],
        });

        assert_eq!(
            audio_codec_string_from_sample_entry(&entry),
            Some("mp4a.40.2".to_owned())
        );
    }

    #[test]
    fn audio_codec_string_from_opus() {
        use shiguredo_mp4::boxes::*;

        let entry = SampleEntry::Opus(OpusBox {
            audio: test_audio_fields(),
            dops_box: DopsBox {
                output_channel_count: 2,
                pre_skip: 312,
                input_sample_rate: 48000,
                output_gain: 0,
            },
            unknown_boxes: vec![],
        });

        assert_eq!(
            audio_codec_string_from_sample_entry(&entry),
            Some("opus".to_owned())
        );
    }

    /// H.265 の SampleEntry は src/video/h265.rs の h265_sample_entry() が生成する値と一致すること
    #[test]
    fn video_codec_string_from_hvc1_sample_entry() {
        use shiguredo_mp4::Uint;
        use shiguredo_mp4::boxes::*;

        // src/video/h265.rs の h265_sample_entry() と同じフィールド値
        let entry = SampleEntry::Hvc1(Hvc1Box {
            visual: test_visual_fields(),
            hvcc_box: HvccBox {
                general_profile_compatibility_flags: 0x60000000,
                general_constraint_indicator_flags: Uint::new(0xb00000000000),
                general_level_idc: 123,
                general_profile_space: Uint::new(0),
                general_tier_flag: Uint::new(0),
                num_temporal_layers: Uint::new(0),
                temporal_id_nested: Uint::new(0),
                min_spatial_segmentation_idc: Uint::new(0),
                parallelism_type: Uint::new(0),
                avg_frame_rate: 30,
                constant_frame_rate: Uint::new(1),
                length_size_minus_one: Uint::new(3),
                nalu_arrays: vec![],
                chroma_format_idc: Uint::new(1),
                general_profile_idc: Uint::new(1),
                bit_depth_luma_minus8: Uint::new(0),
                bit_depth_chroma_minus8: Uint::new(0),
            },
            unknown_boxes: vec![],
        });

        let codec_str = video_codec_string_from_sample_entry(&entry);
        assert!(codec_str.is_some(), "Hvc1 should produce a codec string");
        let codec_str = codec_str.expect("infallible");
        // Hvc1 は "hvc1" プレフィックスであること（hev1 ではない）
        assert!(
            codec_str.starts_with("hvc1."),
            "Hvc1 box must produce hvc1 prefix, got: {codec_str}"
        );
        // profile_idc=1 であること
        assert!(
            codec_str.starts_with("hvc1.1."),
            "profile_idc must be 1, got: {codec_str}"
        );
    }

    /// AV1 の SampleEntry は src/video/av1.rs の av1_sample_entry() が生成する値と一致すること
    #[test]
    fn video_codec_string_from_av01_sample_entry() {
        use shiguredo_mp4::Uint;
        use shiguredo_mp4::boxes::*;

        // src/video/av1.rs の av1_sample_entry() と同じフィールド値
        let entry = SampleEntry::Av01(Av01Box {
            visual: test_visual_fields(),
            av1c_box: Av1cBox {
                seq_profile: Uint::new(0),
                seq_level_idx_0: Uint::new(0),
                seq_tier_0: Uint::new(0),
                high_bitdepth: Uint::new(0),
                twelve_bit: Uint::new(0),
                monochrome: Uint::new(0),
                chroma_subsampling_x: Uint::new(1),
                chroma_subsampling_y: Uint::new(1),
                chroma_sample_position: Uint::new(0),
                initial_presentation_delay_minus_one: None,
                config_obus: vec![],
            },
            unknown_boxes: vec![],
        });

        assert_eq!(
            video_codec_string_from_sample_entry(&entry),
            // Profile 0, Level 0, Main Tier, 8-bit
            Some("av01.0.00M.08".to_owned())
        );
    }

    /// VP9 の SampleEntry は src/encoder/libvpx.rs の vp9_sample_entry() が生成する値と一致すること
    #[test]
    fn video_codec_string_from_vp09_sample_entry() {
        use shiguredo_mp4::Uint;
        use shiguredo_mp4::boxes::*;

        // src/encoder/libvpx.rs の vp9_sample_entry() と同じフィールド値
        let entry = SampleEntry::Vp09(Vp09Box {
            visual: test_visual_fields(),
            vpcc_box: VpccBox {
                profile: 0,
                level: 0,
                bit_depth: Uint::new(8),
                chroma_subsampling: Uint::new(1),
                video_full_range_flag: Uint::new(0),
                colour_primaries: 1,
                transfer_characteristics: 1,
                matrix_coefficients: 1,
                codec_initialization_data: vec![],
            },
            unknown_boxes: vec![],
        });

        assert_eq!(
            video_codec_string_from_sample_entry(&entry),
            // Profile 0, Level 0, 8-bit
            Some("vp09.00.00.08".to_owned())
        );
    }

    /// from_sample_entries() がビデオとオーディオの SampleEntry から正確な CodecString を生成すること
    #[test]
    fn from_sample_entries_hvc1_aac() {
        use shiguredo_mp4::Uint;
        use shiguredo_mp4::boxes::*;

        let video = SampleEntry::Hvc1(Hvc1Box {
            visual: test_visual_fields(),
            hvcc_box: HvccBox {
                general_profile_compatibility_flags: 0x60000000,
                general_constraint_indicator_flags: Uint::new(0xb00000000000),
                general_level_idc: 123,
                general_profile_space: Uint::new(0),
                general_tier_flag: Uint::new(0),
                num_temporal_layers: Uint::new(0),
                temporal_id_nested: Uint::new(0),
                min_spatial_segmentation_idc: Uint::new(0),
                parallelism_type: Uint::new(0),
                avg_frame_rate: 30,
                constant_frame_rate: Uint::new(1),
                length_size_minus_one: Uint::new(3),
                nalu_arrays: vec![],
                chroma_format_idc: Uint::new(1),
                general_profile_idc: Uint::new(1),
                bit_depth_luma_minus8: Uint::new(0),
                bit_depth_chroma_minus8: Uint::new(0),
            },
            unknown_boxes: vec![],
        });

        let audio = SampleEntry::Mp4a(Mp4aBox {
            audio: test_audio_fields(),
            esds_box: EsdsBox {
                es: shiguredo_mp4::descriptors::EsDescriptor {
                    es_id: 1,
                    stream_priority: Uint::new(0),
                    depends_on_es_id: None,
                    url_string: None,
                    ocr_es_id: None,
                    dec_config_descr: shiguredo_mp4::descriptors::DecoderConfigDescriptor {
                        object_type_indication: 0x40,
                        stream_type: Uint::new(0x05),
                        up_stream: Uint::new(0),
                        buffer_size_db: Uint::new(0),
                        max_bitrate: 128000,
                        avg_bitrate: 128000,
                        dec_specific_info: Some(shiguredo_mp4::descriptors::DecoderSpecificInfo {
                            payload: vec![0x11, 0x90],
                        }),
                    },
                    sl_config_descr: shiguredo_mp4::descriptors::SlConfigDescriptor,
                },
            },
            unknown_boxes: vec![],
        });

        let cs = CodecString::from_sample_entries(&video, &audio);
        assert!(cs.is_some(), "Hvc1 + Mp4a should produce a CodecString");
        let cs = cs.expect("infallible");
        assert!(cs.video.starts_with("hvc1."), "video should be hvc1");
        assert_eq!(cs.audio, "mp4a.40.2");
    }

    #[test]
    fn video_codec_string_returns_none_for_audio() {
        use shiguredo_mp4::boxes::*;

        let entry = SampleEntry::Opus(OpusBox {
            audio: test_audio_fields(),
            dops_box: DopsBox {
                output_channel_count: 2,
                pre_skip: 312,
                input_sample_rate: 48000,
                output_gain: 0,
            },
            unknown_boxes: vec![],
        });

        assert_eq!(video_codec_string_from_sample_entry(&entry), None);
    }
}
