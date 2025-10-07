#![cfg_attr(not(feature = "nvcodec"), expect(unused_variables, unused_mut))]

#[cfg(feature = "nvcodec")]
use crate::decoder_nvcodec_params;
use crate::layout::DEFAULT_LAYOUT_JSON;

#[derive(Debug, Clone)]
pub struct LayoutDecodeParams {
    #[cfg(feature = "nvcodec")]
    pub nvcodec_h264: shiguredo_nvcodec::DecoderConfig,
    #[cfg(feature = "nvcodec")]
    pub nvcodec_h265: shiguredo_nvcodec::DecoderConfig,
    #[cfg(feature = "nvcodec")]
    pub nvcodec_av1: shiguredo_nvcodec::DecoderConfig,
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for LayoutDecodeParams {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        let mut params = Self::default();
        for (key, value) in value.to_object()? {
            match &*key.to_unquoted_string_str()? {
                #[cfg(feature = "nvcodec")]
                "nvcodec_h264_decode_params" => {
                    params.nvcodec_h264 = decoder_nvcodec_params::parse_h264_decode_params(value)?;
                }
                #[cfg(feature = "nvcodec")]
                "nvcodec_h265_decode_params" => {
                    params.nvcodec_h265 = decoder_nvcodec_params::parse_h265_decode_params(value)?;
                }
                #[cfg(feature = "nvcodec")]
                "nvcodec_av1_decode_params" => {
                    params.nvcodec_av1 = decoder_nvcodec_params::parse_av1_decode_params(value)?;
                }
                _ => {}
            }
        }
        Ok(params)
    }
}

impl LayoutDecodeParams {
    fn new_from_default_layout() -> Result<Self, nojson::JsonParseError> {
        let default_layout = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
        let value = default_layout.value();

        #[cfg(feature = "nvcodec")]
        let nvcodec_h264 = decoder_nvcodec_params::parse_h264_decode_params(
            value.to_member("nvcodec_h264_decode_params")?.required()?,
        )?;

        #[cfg(feature = "nvcodec")]
        let nvcodec_h265 = decoder_nvcodec_params::parse_h265_decode_params(
            value.to_member("nvcodec_h265_decode_params")?.required()?,
        )?;

        #[cfg(feature = "nvcodec")]
        let nvcodec_av1 = decoder_nvcodec_params::parse_av1_decode_params(
            value.to_member("nvcodec_av1_decode_params")?.required()?,
        )?;

        Ok(Self {
            #[cfg(feature = "nvcodec")]
            nvcodec_h264,
            #[cfg(feature = "nvcodec")]
            nvcodec_h265,
            #[cfg(feature = "nvcodec")]
            nvcodec_av1,
        })
    }
}

impl Default for LayoutDecodeParams {
    fn default() -> Self {
        Self::new_from_default_layout().expect("bug")
    }
}
