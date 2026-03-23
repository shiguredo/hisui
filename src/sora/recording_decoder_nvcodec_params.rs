// Sora の録画ファイル合成処理固有モジュール（recording_ がつかないモジュールからこのモジュールは参照しないこと）
use crate::sora::recording_layout::DEFAULT_LAYOUT_JSON;

pub fn parse_h264_decode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_nvcodec::DecoderConfig, nojson::JsonParseError> {
    let mut config = default_decoder_config(shiguredo_nvcodec::DecoderCodec::H264);

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = default
        .value()
        .to_member("nvcodec_h264_decode_params")?
        .required()?;
    update_decode_params(params, &mut config)?;

    // 実際のレイアウトの設定を反映
    update_decode_params(value, &mut config)?;

    Ok(config)
}

pub fn parse_h265_decode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_nvcodec::DecoderConfig, nojson::JsonParseError> {
    let mut config = default_decoder_config(shiguredo_nvcodec::DecoderCodec::Hevc);

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = default
        .value()
        .to_member("nvcodec_h265_decode_params")?
        .required()?;
    update_decode_params(params, &mut config)?;

    // 実際のレイアウトの設定を反映
    update_decode_params(value, &mut config)?;

    Ok(config)
}

pub fn parse_av1_decode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_nvcodec::DecoderConfig, nojson::JsonParseError> {
    let mut config = default_decoder_config(shiguredo_nvcodec::DecoderCodec::Av1);

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = default
        .value()
        .to_member("nvcodec_av1_decode_params")?
        .required()?;
    update_decode_params(params, &mut config)?;

    // 実際のレイアウトの設定を反映
    update_decode_params(value, &mut config)?;

    Ok(config)
}

pub fn parse_vp8_decode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_nvcodec::DecoderConfig, nojson::JsonParseError> {
    let mut config = default_decoder_config(shiguredo_nvcodec::DecoderCodec::Vp8);

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = default
        .value()
        .to_member("nvcodec_vp8_decode_params")?
        .required()?;
    update_decode_params(params, &mut config)?;

    // 実際のレイアウトの設定を反映
    update_decode_params(value, &mut config)?;

    Ok(config)
}

pub fn parse_vp9_decode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_nvcodec::DecoderConfig, nojson::JsonParseError> {
    let mut config = default_decoder_config(shiguredo_nvcodec::DecoderCodec::Vp9);

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = default
        .value()
        .to_member("nvcodec_vp9_decode_params")?
        .required()?;
    update_decode_params(params, &mut config)?;

    // 実際のレイアウトの設定を反映
    update_decode_params(value, &mut config)?;

    Ok(config)
}

fn update_decode_params(
    params: nojson::RawJsonValue<'_, '_>,
    config: &mut shiguredo_nvcodec::DecoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // デバイスID
    if let Some(v) = params.to_member("device_id")?.optional() {
        config.device_id = v.try_into()?;
    }

    // デコード用サーフェスの最大数
    if let Some(v) = params.to_member("max_num_decode_surfaces")?.optional() {
        config.max_num_decode_surfaces = v.try_into()?;
    }

    // 表示遅延
    if let Some(v) = params.to_member("max_display_delay")?.optional() {
        config.max_display_delay = v.try_into()?;
    }

    Ok(())
}

fn default_decoder_config(
    codec: shiguredo_nvcodec::DecoderCodec,
) -> shiguredo_nvcodec::DecoderConfig {
    shiguredo_nvcodec::DecoderConfig {
        codec,
        device_id: 0,
        max_num_decode_surfaces: 20,
        max_display_delay: 0,
        surface_format: shiguredo_nvcodec::SurfaceFormat::Nv12,
    }
}
