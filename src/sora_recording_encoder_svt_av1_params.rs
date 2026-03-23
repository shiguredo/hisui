use crate::sora_recording_layout::DEFAULT_LAYOUT_JSON;

pub fn parse_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_svt_av1::EncoderConfig, nojson::JsonParseError> {
    // width / height は後で上書きされるのでダミー値で初期化する
    let mut config =
        shiguredo_svt_av1::EncoderConfig::new(0, 0, shiguredo_svt_av1::ColorFormat::I420);

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

/// JSON の値を u8 に変換する（Boolean の場合は false=0, true=1 として扱う）
fn to_u8(v: nojson::RawJsonValue<'_, '_>) -> Result<u8, nojson::JsonParseError> {
    if let Ok(b) = <bool as TryFrom<nojson::RawJsonValue<'_, '_>>>::try_from(v) {
        Ok(u8::from(b))
    } else {
        v.try_into()
    }
}

/// JSON の値を i32 に変換する（Boolean の場合は false=0, true=1 として扱う）
fn to_i32(v: nojson::RawJsonValue<'_, '_>) -> Result<i32, nojson::JsonParseError> {
    if let Ok(b) = <bool as TryFrom<nojson::RawJsonValue<'_, '_>>>::try_from(v) {
        Ok(i32::from(b))
    } else {
        v.try_into()
    }
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
    // - target_bit_rate

    // [NOTE] 以下は外部 crate 化で削除されたフィールドなので無視する:
    // - pred_structure
    // - pin_threads
    // - target_socket
    // - enable_tpl_la
    // - force_key_frames
    // - recon_enabled
    // - encoder_bit_depth
    // - encoder_color_format
    // - profile
    // - level
    // - tier

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
            "cqp_or_crf" => shiguredo_svt_av1::RcMode::CqpOrCrf,
            "vbr" => shiguredo_svt_av1::RcMode::Vbr,
            "cbr" => shiguredo_svt_av1::RcMode::Cbr,
            _ => return Err(v.invalid("unknown 'rate_control_mode' value")),
        };
    }

    if let Some(v) = params.to_member("max_bit_rate")?.optional() {
        config.max_bit_rate = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("over_shoot_pct")?.optional() {
        config.over_shoot_pct = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("under_shoot_pct")?.optional() {
        config.under_shoot_pct = Some(v.try_into()?);
    }

    // === GOP・フレーム構造関連 ===
    if let Some(v) = params.to_member("intra_period_length")?.optional() {
        config.intra_period_length = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("hierarchical_levels")?.optional() {
        config.hierarchical_levels = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("scene_change_detection")?.optional() {
        config.scene_change_detection = v.try_into()?;
    }
    if let Some(v) = params.to_member("look_ahead_distance")?.optional() {
        config.look_ahead_distance = Some(v.try_into()?);
    }

    // === 並列処理関連 ===
    if let Some(v) = params.to_member("tile_columns")?.optional() {
        config.tile_columns = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("tile_rows")?.optional() {
        config.tile_rows = Some(v.try_into()?);
    }

    // === フィルタリング関連 ===
    // 旧 API では整数型と Boolean が混在していたため、Boolean も整数として受け付ける
    if let Some(v) = params.to_member("enable_dlf_flag")?.optional() {
        config.enable_dlf_flag = Some(to_u8(v)?);
    }
    if let Some(v) = params.to_member("cdef_level")?.optional() {
        config.cdef_level = Some(to_i32(v)?);
    }
    if let Some(v) = params.to_member("enable_restoration_filtering")?.optional() {
        config.enable_restoration_filtering = Some(to_i32(v)?);
    }

    // === 高度な設定 ===
    if let Some(v) = params.to_member("enable_tf")?.optional() {
        config.enable_tf = Some(to_u8(v)?);
    }
    if let Some(v) = params.to_member("enable_overlays")?.optional() {
        config.enable_overlays = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("film_grain_denoise_strength")?.optional() {
        config.film_grain_denoise_strength = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("stat_report")?.optional() {
        config.stat_report = v.try_into()?;
    }

    // === エンコーダー固有設定 ===
    if let Some(v) = params.to_member("color_format")?.optional() {
        config.color_format = match v.to_unquoted_string_str()?.as_ref() {
            "i420" => shiguredo_svt_av1::ColorFormat::I420,
            "i42010" => shiguredo_svt_av1::ColorFormat::I42010,
            _ => return Err(v.invalid("unknown 'color_format' value")),
        };
    }

    if let Some(v) = params.to_member("fast_decode")?.optional() {
        config.fast_decode = Some(to_u8(v)?);
    }

    Ok(())
}
