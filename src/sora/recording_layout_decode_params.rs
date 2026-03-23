#![cfg_attr(not(feature = "nvcodec"), expect(unused_variables, unused_mut))]

use crate::decoder::DecodeConfig;
#[cfg(feature = "nvcodec")]
use crate::sora::recording_decoder_nvcodec_params;
use crate::sora::recording_layout::DEFAULT_LAYOUT_JSON;

#[derive(Debug, Clone)]
pub struct LayoutDecodeParams {
    pub config: DecodeConfig,
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for LayoutDecodeParams {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        let mut config = Self::default().config;
        for (key, value) in value.to_object()? {
            match &*key.to_unquoted_string_str()? {
                #[cfg(feature = "nvcodec")]
                "nvcodec_h264_decode_params" => {
                    config.nvcodec_h264 =
                        recording_decoder_nvcodec_params::parse_h264_decode_params(value)?;
                }
                #[cfg(feature = "nvcodec")]
                "nvcodec_h265_decode_params" => {
                    config.nvcodec_h265 =
                        recording_decoder_nvcodec_params::parse_h265_decode_params(value)?;
                }
                #[cfg(feature = "nvcodec")]
                "nvcodec_av1_decode_params" => {
                    config.nvcodec_av1 =
                        recording_decoder_nvcodec_params::parse_av1_decode_params(value)?;
                }
                #[cfg(feature = "nvcodec")]
                "nvcodec_vp8_decode_params" => {
                    config.nvcodec_vp8 =
                        recording_decoder_nvcodec_params::parse_vp8_decode_params(value)?;
                }
                #[cfg(feature = "nvcodec")]
                "nvcodec_vp9_decode_params" => {
                    config.nvcodec_vp9 =
                        recording_decoder_nvcodec_params::parse_vp9_decode_params(value)?;
                }
                _ => {}
            }
        }
        Ok(Self { config })
    }
}

impl LayoutDecodeParams {
    fn new_config_from_default_layout() -> Result<DecodeConfig, nojson::JsonParseError> {
        let default_layout = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
        let value = default_layout.value();

        #[cfg(feature = "nvcodec")]
        let nvcodec_h264 = recording_decoder_nvcodec_params::parse_h264_decode_params(
            value.to_member("nvcodec_h264_decode_params")?.required()?,
        )?;

        #[cfg(feature = "nvcodec")]
        let nvcodec_h265 = recording_decoder_nvcodec_params::parse_h265_decode_params(
            value.to_member("nvcodec_h265_decode_params")?.required()?,
        )?;

        #[cfg(feature = "nvcodec")]
        let nvcodec_av1 = recording_decoder_nvcodec_params::parse_av1_decode_params(
            value.to_member("nvcodec_av1_decode_params")?.required()?,
        )?;

        #[cfg(feature = "nvcodec")]
        let nvcodec_vp8 = recording_decoder_nvcodec_params::parse_vp8_decode_params(
            value.to_member("nvcodec_vp8_decode_params")?.required()?,
        )?;

        #[cfg(feature = "nvcodec")]
        let nvcodec_vp9 = recording_decoder_nvcodec_params::parse_vp9_decode_params(
            value.to_member("nvcodec_vp9_decode_params")?.required()?,
        )?;

        Ok(DecodeConfig {
            #[cfg(feature = "nvcodec")]
            nvcodec_h264,
            #[cfg(feature = "nvcodec")]
            nvcodec_h265,
            #[cfg(feature = "nvcodec")]
            nvcodec_av1,
            #[cfg(feature = "nvcodec")]
            nvcodec_vp8,
            #[cfg(feature = "nvcodec")]
            nvcodec_vp9,
        })
    }
}

impl Default for LayoutDecodeParams {
    fn default() -> Self {
        Self {
            config: Self::new_config_from_default_layout().expect("bug"),
        }
    }
}
