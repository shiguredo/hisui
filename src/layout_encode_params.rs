use crate::{encoder_libvpx_params, encoder_openh264_params, encoder_svt_av1_params};

#[derive(Debug, Clone, Default)]
pub struct LayoutEncodeParams {
    // TODO: bitrate とかはこっちに移動してもいい
    pub libvpx_vp8: Option<shiguredo_libvpx::EncoderConfig>,
    pub libvpx_vp9: Option<shiguredo_libvpx::EncoderConfig>,
    pub openh264: Option<shiguredo_openh264::EncoderConfig>,
    pub svt_av1: Option<shiguredo_svt_av1::EncoderConfig>,
}

impl<'text> nojson::FromRawJsonValue<'text> for LayoutEncodeParams {
    fn from_raw_json_value(
        value: nojson::RawJsonValue<'text, '_>,
    ) -> Result<Self, nojson::JsonParseError> {
        let mut params = Self::default();
        for (key, value) in value.to_object()? {
            match &*key.to_unquoted_string_str()? {
                "libvpx_vp8_encode_params" => {
                    params.libvpx_vp8 =
                        Some(encoder_libvpx_params::parse_vp8_encode_params(value)?);
                }
                "libvpx_vp9_encode_params" => {
                    params.libvpx_vp9 =
                        Some(encoder_libvpx_params::parse_vp9_encode_params(value)?);
                }
                "openh264_encode_params" => {
                    params.openh264 = Some(encoder_openh264_params::parse_encode_params(value)?);
                }
                "svt_av1_encode_params" => {
                    params.svt_av1 = Some(encoder_svt_av1_params::parse_encode_params(value)?);
                }
                _ => {}
            }
        }
        Ok(params)
    }
}
