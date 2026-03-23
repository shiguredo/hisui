use crate::sora::recording_layout::DEFAULT_LAYOUT_JSON;

pub fn parse_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_openh264::EncoderConfig, nojson::JsonParseError> {
    // width / height / target_bitrate / fps は後で実値に上書きするため、ここではダミー値を使う。
    let mut config = shiguredo_openh264::EncoderConfig::new(1, 1, 1, 1, 1);

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = default
        .value()
        .to_member("openh264_encode_params")?
        .required()?;
    update_encode_params(params, &mut config)?;

    // 実際のレイアウトの設定を反映
    update_encode_params(value, &mut config)?;

    Ok(config)
}

fn update_encode_params(
    params: nojson::RawJsonValue<'_, '_>,
    config: &mut shiguredo_openh264::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - fps_numerator
    // - fps_denominator
    // - target_bitrate

    // 基本的なエンコーダーパラメーター
    if let Some(v) = params.to_member("max_qp")?.optional() {
        config.max_qp = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("min_qp")?.optional() {
        config.min_qp = Some(v.try_into()?);
    }

    // 複雑度モード
    if let Some(v) = params.to_member("complexity_mode")?.optional() {
        config.complexity_mode = Some(match v.to_unquoted_string_str()?.as_ref() {
            "low" => shiguredo_openh264::ComplexityMode::Low,
            "medium" => shiguredo_openh264::ComplexityMode::Medium,
            "high" => shiguredo_openh264::ComplexityMode::High,
            _ => return Err(v.invalid("unknown 'complexity_mode' value")),
        });
    }

    // エントロピー符号化モード
    if let Some(v) = params.to_member("entropy_coding_mode")?.optional() {
        config.entropy_coding_mode = Some(match v.to_unquoted_string_str()?.as_ref() {
            "cavlc" => shiguredo_openh264::EntropyCodingMode::Cavlc,
            "cabac" => shiguredo_openh264::EntropyCodingMode::Cabac,
            _ => return Err(v.invalid("unknown 'entropy_coding_mode' value")),
        });
    }

    // 互換のため、旧キー `entropy_coding` (bool) も受け付ける。
    if config.entropy_coding_mode.is_none()
        && let Some(v) = params.to_member("entropy_coding")?.optional()
    {
        let enabled: bool = v.try_into()?;
        config.entropy_coding_mode = Some(if enabled {
            shiguredo_openh264::EntropyCodingMode::Cabac
        } else {
            shiguredo_openh264::EntropyCodingMode::Cavlc
        });
    }

    // 参照フレーム数
    if let Some(v) = params.to_member("ref_frame_count")?.optional() {
        config.ref_frame_count = Some(v.try_into()?);
    }

    // スレッド数
    if let Some(v) = params.to_member("thread_count")?.optional() {
        config.thread_count = Some(v.try_into()?);
    }

    // 空間レイヤー数
    if let Some(v) = params.to_member("spatial_layers")?.optional() {
        config.spatial_layers = Some(v.try_into()?);
    }

    // 時間レイヤー数
    if let Some(v) = params.to_member("temporal_layers")?.optional() {
        config.temporal_layers = Some(v.try_into()?);
    }

    // Intra フレーム間隔
    if let Some(v) = params.to_member("intra_period")?.optional() {
        config.intra_period = Some(v.try_into()?);
    }

    // レート制御モード
    if let Some(v) = params.to_member("rate_control_mode")?.optional() {
        config.rate_control_mode = Some(match v.to_unquoted_string_str()?.as_ref() {
            "off" => shiguredo_openh264::RateControlMode::Off,
            "quality" => shiguredo_openh264::RateControlMode::Quality,
            "bitrate" => shiguredo_openh264::RateControlMode::Bitrate,
            "timestamp" => shiguredo_openh264::RateControlMode::Timestamp,
            _ => return Err(v.invalid("unknown 'rate_control_mode' value")),
        });
    }

    // 前処理機能設定
    if let Some(v) = params.to_member("denoise")?.optional() {
        config.denoise = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("background_detection")?.optional() {
        config.background_detection = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("adaptive_quantization")?.optional() {
        config.adaptive_quantization = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("scene_change_detection")?.optional() {
        config.scene_change_detection = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("deblocking_filter")?.optional() {
        config.deblocking_filter = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("long_term_reference")?.optional() {
        config.long_term_reference = Some(v.try_into()?);
    }

    // スライスモード
    if let Some(v) = params.to_member("slice_mode")?.optional() {
        let mode_type: String = v.to_member("type")?.required()?.try_into()?;
        config.slice_mode = Some(match mode_type.as_str() {
            "single" => shiguredo_openh264::SliceMode::Single,
            "fixed_count" => {
                let count = v.to_member("count")?.required()?.try_into()?;
                shiguredo_openh264::SliceMode::FixedCount(count)
            }
            "size_constrained" => {
                let size = v.to_member("size")?.required()?.try_into()?;
                shiguredo_openh264::SliceMode::SizeConstrained(size)
            }
            _ => return Err(v.invalid("unknown 'slice_mode.type' value")),
        });
    }

    Ok(())
}
