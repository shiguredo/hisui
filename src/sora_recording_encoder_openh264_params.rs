use crate::json::JsonObject;
use crate::sora_recording_layout::DEFAULT_LAYOUT_JSON;

pub fn parse_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_openh264::EncoderConfig, nojson::JsonParseError> {
    // width / height / target_bitrate / fps は後で実値に上書きするため、ここではダミー値を使う。
    let mut config = shiguredo_openh264::EncoderConfig::new(1, 1, 1, 1, 1);

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = JsonObject::new(
        default
            .value()
            .to_member("openh264_encode_params")?
            .required()?,
    )?;
    update_encode_params(params, &mut config)?;

    // 実際のレイアウトの設定を反映
    let params = JsonObject::new(value)?;
    update_encode_params(params, &mut config)?;

    Ok(config)
}

fn update_encode_params(
    params: JsonObject<'_, '_>,
    config: &mut shiguredo_openh264::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - fps_numerator
    // - fps_denominator
    // - target_bitrate

    // 基本的なエンコーダーパラメーター
    config.max_qp = params.get("max_qp")?.or(config.max_qp);
    config.min_qp = params.get("min_qp")?.or(config.min_qp);

    // 複雑度モード
    config.complexity_mode = params
        .get_with("complexity_mode", |v| {
            match v.to_unquoted_string_str()?.as_ref() {
                "low" => Ok(shiguredo_openh264::ComplexityMode::Low),
                "medium" => Ok(shiguredo_openh264::ComplexityMode::Medium),
                "high" => Ok(shiguredo_openh264::ComplexityMode::High),
                _ => Err(v.invalid("unknown 'complexity_mode' value")),
            }
        })?
        .or(config.complexity_mode);

    // エントロピー符号化モード
    config.entropy_coding_mode = params
        .get_with("entropy_coding_mode", |v| {
            match v.to_unquoted_string_str()?.as_ref() {
                "cavlc" => Ok(shiguredo_openh264::EntropyCodingMode::Cavlc),
                "cabac" => Ok(shiguredo_openh264::EntropyCodingMode::Cabac),
                _ => Err(v.invalid("unknown 'entropy_coding_mode' value")),
            }
        })?
        .or(config.entropy_coding_mode);

    // 互換のため、旧キー `entropy_coding` (bool) も受け付ける。
    if config.entropy_coding_mode.is_none() {
        config.entropy_coding_mode = params
            .get_with("entropy_coding", |v| {
                let enabled: bool = v.try_into()?;
                Ok(if enabled {
                    shiguredo_openh264::EntropyCodingMode::Cabac
                } else {
                    shiguredo_openh264::EntropyCodingMode::Cavlc
                })
            })?
            .or(config.entropy_coding_mode);
    }

    // 参照フレーム数
    config.ref_frame_count = params.get("ref_frame_count")?.or(config.ref_frame_count);

    // スレッド数
    config.thread_count = params.get("thread_count")?.or(config.thread_count);

    // 空間レイヤー数
    config.spatial_layers = params.get("spatial_layers")?.or(config.spatial_layers);

    // 時間レイヤー数
    config.temporal_layers = params.get("temporal_layers")?.or(config.temporal_layers);

    // Intra フレーム間隔
    config.intra_period = params.get("intra_period")?.or(config.intra_period);

    // レート制御モード
    config.rate_control_mode = params
        .get_with("rate_control_mode", |v| {
            match v.to_unquoted_string_str()?.as_ref() {
                "off" => Ok(shiguredo_openh264::RateControlMode::Off),
                "quality" => Ok(shiguredo_openh264::RateControlMode::Quality),
                "bitrate" => Ok(shiguredo_openh264::RateControlMode::Bitrate),
                "timestamp" => Ok(shiguredo_openh264::RateControlMode::Timestamp),
                _ => Err(v.invalid("unknown 'rate_control_mode' value")),
            }
        })?
        .or(config.rate_control_mode);

    // 前処理機能設定
    config.denoise = params.get("denoise")?.or(config.denoise);
    config.background_detection = params
        .get("background_detection")?
        .or(config.background_detection);
    config.adaptive_quantization = params
        .get("adaptive_quantization")?
        .or(config.adaptive_quantization);
    config.scene_change_detection = params
        .get("scene_change_detection")?
        .or(config.scene_change_detection);
    config.deblocking_filter = params
        .get("deblocking_filter")?
        .or(config.deblocking_filter);
    config.long_term_reference = params
        .get("long_term_reference")?
        .or(config.long_term_reference);

    // スライスモード
    config.slice_mode = params
        .get_with("slice_mode", |v| {
            let slice_obj = JsonObject::new(v)?;
            let mode_type: String = slice_obj.get_required("type")?;
            match mode_type.as_str() {
                "single" => Ok(shiguredo_openh264::SliceMode::Single),
                "fixed_count" => {
                    let count = slice_obj.get_required("count")?;
                    Ok(shiguredo_openh264::SliceMode::FixedCount(count))
                }
                "size_constrained" => {
                    let size = slice_obj.get_required("size")?;
                    Ok(shiguredo_openh264::SliceMode::SizeConstrained(size))
                }
                _ => Err(v.invalid("unknown 'slice_mode.type' value")),
            }
        })?
        .or(config.slice_mode);

    Ok(())
}
