#[cfg(target_os = "macos")]
use crate::encoder_video_toolbox_params;
use crate::{encoder_libvpx_params, encoder_openh264_params, encoder_svt_av1_params};

#[derive(Debug, Clone, Default)]
pub struct LayoutEncodeParams {
    // TODO: bitrate とかはこっちに移動してもいい
    pub libvpx_vp8: Option<shiguredo_libvpx::EncoderConfig>,
    pub libvpx_vp9: Option<shiguredo_libvpx::EncoderConfig>,
    pub openh264: Option<shiguredo_openh264::EncoderConfig>,
    pub svt_av1: Option<shiguredo_svt_av1::EncoderConfig>,
    #[cfg(target_os = "macos")]
    pub video_toolbox_h264: Option<shiguredo_video_toolbox::EncoderConfig>,
    #[cfg(target_os = "macos")]
    pub video_toolbox_h265: Option<shiguredo_video_toolbox::EncoderConfig>,
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
                #[cfg(target_os = "macos")]
                "video_toolbox_h264_encode_params" => {
                    params.video_toolbox_h264 = Some(
                        encoder_video_toolbox_params::parse_h264_encode_params(value)?,
                    );
                }
                #[cfg(target_os = "macos")]
                "video_toolbox_h265_encode_params" => {
                    params.video_toolbox_h265 = Some(
                        encoder_video_toolbox_params::parse_h265_encode_params(value)?,
                    );
                }
                _ => {}
            }
        }
        Ok(params)
    }
}
