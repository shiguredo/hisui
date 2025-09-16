use crate::json::JsonObject;
use crate::layout::DEFAULT_LAYOUT_JSON;

pub fn parse_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_svt_av1::EncoderConfig, nojson::JsonParseError> {
    let mut config = shiguredo_svt_av1::EncoderConfig::default();

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = JsonObject::new(
        default
            .value()
            .to_member("svt_av1_encode_params")?
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
    config: &mut shiguredo_svt_av1::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - fps_numerator
    // - fps_denominator
    // - target_bitrate

    // === 品質・速度制御関連 ===
    config.enc_mode = params.get("enc_mode")?.unwrap_or(config.enc_mode);
    config.qp = params.get("qp")?.or(config.qp);
    config.min_qp_allowed = params.get("min_qp_allowed")?.or(config.min_qp_allowed);
    config.max_qp_allowed = params.get("max_qp_allowed")?.or(config.max_qp_allowed);

    // === レート制御関連 ===
    config.rate_control_mode = params
        .get_with("rate_control_mode", |v| {
            match v.to_unquoted_string_str()?.as_ref() {
                "cqp_or_crf" => Ok(shiguredo_svt_av1::RateControlMode::CqpOrCrf),
                "vbr" => Ok(shiguredo_svt_av1::RateControlMode::Vbr),
                "cbr" => Ok(shiguredo_svt_av1::RateControlMode::Cbr),
                _ => Err(v.invalid("unknown 'rate_control_mode' value")),
            }
        })?
        .unwrap_or(config.rate_control_mode);

    config.max_bit_rate = params.get("max_bit_rate")?.or(config.max_bit_rate);
    config.over_shoot_pct = params
        .get("over_shoot_pct")?
        .unwrap_or(config.over_shoot_pct);
    config.under_shoot_pct = params
        .get("under_shoot_pct")?
        .unwrap_or(config.under_shoot_pct);

    // === GOP・フレーム構造関連 ===
    config.intra_period_length = params
        .get("intra_period_length")?
        .unwrap_or(config.intra_period_length);
    config.hierarchical_levels = params
        .get("hierarchical_levels")?
        .unwrap_or(config.hierarchical_levels);
    config.pred_structure = params
        .get("pred_structure")?
        .unwrap_or(config.pred_structure);
    config.scene_change_detection = params
        .get("scene_change_detection")?
        .unwrap_or(config.scene_change_detection);
    config.look_ahead_distance = params
        .get("look_ahead_distance")?
        .unwrap_or(config.look_ahead_distance);

    // === 並列処理関連 ===
    config.pin_threads = params.get("pin_threads")?.or(config.pin_threads);
    config.tile_columns = params.get("tile_columns")?.or(config.tile_columns);
    config.tile_rows = params.get("tile_rows")?.or(config.tile_rows);
    config.target_socket = params.get("target_socket")?.unwrap_or(config.target_socket);

    // === フィルタリング関連 ===
    config.enable_dlf_flag = params
        .get("enable_dlf_flag")?
        .unwrap_or(config.enable_dlf_flag);
    config.cdef_level = params.get("cdef_level")?.unwrap_or(config.cdef_level);
    config.enable_restoration_filtering = params
        .get("enable_restoration_filtering")?
        .unwrap_or(config.enable_restoration_filtering);

    // === 高度な設定 ===
    config.enable_tf = params.get("enable_tf")?.unwrap_or(config.enable_tf);
    config.enable_overlays = params
        .get("enable_overlays")?
        .unwrap_or(config.enable_overlays);
    config.film_grain_denoise_strength = params
        .get("film_grain_denoise_strength")?
        .unwrap_or(config.film_grain_denoise_strength);
    config.enable_tpl_la = params.get("enable_tpl_la")?.unwrap_or(config.enable_tpl_la);
    config.force_key_frames = params
        .get("force_key_frames")?
        .unwrap_or(config.force_key_frames);
    config.stat_report = params.get("stat_report")?.unwrap_or(config.stat_report);
    config.recon_enabled = params.get("recon_enabled")?.unwrap_or(config.recon_enabled);

    // === エンコーダー固有設定 ===
    config.encoder_bit_depth = params
        .get("encoder_bit_depth")?
        .unwrap_or(config.encoder_bit_depth);

    config.encoder_color_format = params
        .get_with("encoder_color_format", |v| {
            match v.to_unquoted_string_str()?.as_ref() {
                "yuv400" => Ok(shiguredo_svt_av1::ColorFormat::Yuv400),
                "yuv420" => Ok(shiguredo_svt_av1::ColorFormat::Yuv420),
                "yuv422" => Ok(shiguredo_svt_av1::ColorFormat::Yuv422),
                "yuv444" => Ok(shiguredo_svt_av1::ColorFormat::Yuv444),
                _ => Err(v.invalid("unknown 'encoder_color_format' value")),
            }
        })?
        .unwrap_or(config.encoder_color_format);

    config.profile = params.get("profile")?.unwrap_or(config.profile);
    config.level = params.get("level")?.unwrap_or(config.level);
    config.tier = params.get("tier")?.unwrap_or(config.tier);
    config.fast_decode = params.get("fast_decode")?.unwrap_or(config.fast_decode);

    Ok(())
}
