use crate::json::JsonObject;

pub fn parse_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_openh264::EncoderConfig, nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - fps_numerator
    // - fps_denominator
    // - target_bitrate
    let params = JsonObject::new(value)?;
    let mut config = shiguredo_openh264::EncoderConfig::default();

    // 基本的なエンコーダーパラメーター
    config.max_qp = params.get("max_qp")?.unwrap_or(config.max_qp);
    config.min_qp = params.get("min_qp")?.unwrap_or(config.min_qp);

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
        .unwrap_or(config.complexity_mode);

    // エントロピー符号化モード
    config.entropy_coding = params.get("entropy_coding")?.unwrap_or_default();

    // 参照フレーム数
    config.ref_frame_count = params
        .get("ref_frame_count")?
        .unwrap_or(config.ref_frame_count);

    // スレッド数
    config.thread_count = params.get("thread_count")?;

    // 空間レイヤー数
    config.spatial_layers = params
        .get("spatial_layers")?
        .unwrap_or(config.spatial_layers);

    // 時間レイヤー数
    config.temporal_layers = params
        .get("temporal_layers")?
        .unwrap_or(config.temporal_layers);

    // Intra フレーム間隔
    config.intra_period = params.get("intra_period")?;

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
        .unwrap_or(config.rate_control_mode);

    // 前処理機能設定
    config.denoise = params.get("denoise")?.unwrap_or_default();
    config.background_detection = params.get("background_detection")?.unwrap_or_default();
    config.adaptive_quantization = params.get("adaptive_quantization")?.unwrap_or_default();
    config.scene_change_detection = params.get("scene_change_detection")?.unwrap_or_default();
    config.deblocking_filter = params.get("deblocking_filter")?.unwrap_or_default();
    config.long_term_reference = params.get("long_term_reference")?.unwrap_or_default();

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
        .unwrap_or(config.slice_mode);

    Ok(config)
}
