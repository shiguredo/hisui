use crate::sora_recording_layout::DEFAULT_LAYOUT_JSON;

pub fn parse_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_svt_av1::EncoderConfig, nojson::JsonParseError> {
    let mut config = shiguredo_svt_av1::EncoderConfig::default();

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = default
        .value()
        .to_member("svt_av1_encode_params")?
        .required()?;
    update_encode_params(params, &mut config)?;

    // 実際のレイアウトの設定を反映
    update_encode_params(value, &mut config)?;

    Ok(config)
}

fn update_encode_params(
    params: nojson::RawJsonValue<'_, '_>,
    config: &mut shiguredo_svt_av1::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - fps_numerator
    // - fps_denominator
    // - target_bitrate

    // === 品質・速度制御関連 ===
    if let Some(v) = params.to_member("enc_mode")?.optional() {
        config.enc_mode = v.try_into()?;
    }
    if let Some(v) = params.to_member("qp")?.optional() {
        config.qp = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("min_qp_allowed")?.optional() {
        config.min_qp_allowed = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("max_qp_allowed")?.optional() {
        config.max_qp_allowed = Some(v.try_into()?);
    }

    // === レート制御関連 ===
    if let Some(v) = params.to_member("rate_control_mode")?.optional() {
        config.rate_control_mode = match v.to_unquoted_string_str()?.as_ref() {
            "cqp_or_crf" => shiguredo_svt_av1::RateControlMode::CqpOrCrf,
            "vbr" => shiguredo_svt_av1::RateControlMode::Vbr,
            "cbr" => shiguredo_svt_av1::RateControlMode::Cbr,
            _ => return Err(v.invalid("unknown 'rate_control_mode' value")),
        };
    }

    if let Some(v) = params.to_member("max_bit_rate")?.optional() {
        config.max_bit_rate = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("over_shoot_pct")?.optional() {
        config.over_shoot_pct = v.try_into()?;
    }
    if let Some(v) = params.to_member("under_shoot_pct")?.optional() {
        config.under_shoot_pct = v.try_into()?;
    }

    // === GOP・フレーム構造関連 ===
    if let Some(v) = params.to_member("intra_period_length")?.optional() {
        config.intra_period_length = v.try_into()?;
    }
    if let Some(v) = params.to_member("hierarchical_levels")?.optional() {
        config.hierarchical_levels = v.try_into()?;
    }
    if let Some(v) = params.to_member("pred_structure")?.optional() {
        config.pred_structure = v.try_into()?;
    }
    if let Some(v) = params.to_member("scene_change_detection")?.optional() {
        config.scene_change_detection = v.try_into()?;
    }
    if let Some(v) = params.to_member("look_ahead_distance")?.optional() {
        config.look_ahead_distance = v.try_into()?;
    }

    // === 並列処理関連 ===
    if let Some(v) = params.to_member("pin_threads")?.optional() {
        config.pin_threads = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("tile_columns")?.optional() {
        config.tile_columns = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("tile_rows")?.optional() {
        config.tile_rows = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("target_socket")?.optional() {
        config.target_socket = v.try_into()?;
    }

    // === フィルタリング関連 ===
    if let Some(v) = params.to_member("enable_dlf_flag")?.optional() {
        config.enable_dlf_flag = v.try_into()?;
    }
    if let Some(v) = params.to_member("cdef_level")?.optional() {
        config.cdef_level = v.try_into()?;
    }
    if let Some(v) = params.to_member("enable_restoration_filtering")?.optional() {
        config.enable_restoration_filtering = v.try_into()?;
    }

    // === 高度な設定 ===
    if let Some(v) = params.to_member("enable_tf")?.optional() {
        config.enable_tf = v.try_into()?;
    }
    if let Some(v) = params.to_member("enable_overlays")?.optional() {
        config.enable_overlays = v.try_into()?;
    }
    if let Some(v) = params.to_member("film_grain_denoise_strength")?.optional() {
        config.film_grain_denoise_strength = v.try_into()?;
    }
    if let Some(v) = params.to_member("enable_tpl_la")?.optional() {
        config.enable_tpl_la = v.try_into()?;
    }
    if let Some(v) = params.to_member("force_key_frames")?.optional() {
        config.force_key_frames = v.try_into()?;
    }
    if let Some(v) = params.to_member("stat_report")?.optional() {
        config.stat_report = v.try_into()?;
    }
    if let Some(v) = params.to_member("recon_enabled")?.optional() {
        config.recon_enabled = v.try_into()?;
    }

    // === エンコーダー固有設定 ===
    if let Some(v) = params.to_member("encoder_bit_depth")?.optional() {
        config.encoder_bit_depth = v.try_into()?;
    }

    if let Some(v) = params.to_member("encoder_color_format")?.optional() {
        config.encoder_color_format = match v.to_unquoted_string_str()?.as_ref() {
            "yuv400" => shiguredo_svt_av1::ColorFormat::Yuv400,
            "yuv420" => shiguredo_svt_av1::ColorFormat::Yuv420,
            "yuv422" => shiguredo_svt_av1::ColorFormat::Yuv422,
            "yuv444" => shiguredo_svt_av1::ColorFormat::Yuv444,
            _ => return Err(v.invalid("unknown 'encoder_color_format' value")),
        };
    }

    if let Some(v) = params.to_member("profile")?.optional() {
        config.profile = v.try_into()?;
    }
    if let Some(v) = params.to_member("level")?.optional() {
        config.level = v.try_into()?;
    }
    if let Some(v) = params.to_member("tier")?.optional() {
        config.tier = v.try_into()?;
    }
    if let Some(v) = params.to_member("fast_decode")?.optional() {
        config.fast_decode = v.try_into()?;
    }

    Ok(())
}
