#[cfg(target_os = "macos")]
use crate::encoder_video_toolbox_params;
use crate::{encoder_libvpx_params, encoder_openh264_params, encoder_svt_av1_params};

#[derive(Debug, Clone)]
pub struct LayoutEncodeParams {
    pub libvpx_vp8: shiguredo_libvpx::EncoderConfig,
    pub libvpx_vp9: shiguredo_libvpx::EncoderConfig,
    pub openh264: shiguredo_openh264::EncoderConfig,
    pub svt_av1: shiguredo_svt_av1::EncoderConfig,
    #[cfg(target_os = "macos")]
    pub video_toolbox_h264: shiguredo_video_toolbox::EncoderConfig,
    #[cfg(target_os = "macos")]
    pub video_toolbox_h265: shiguredo_video_toolbox::EncoderConfig,
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for LayoutEncodeParams {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        let mut params = Self::default();
        for (key, value) in value.to_object()? {
            match &*key.to_unquoted_string_str()? {
                "libvpx_vp8_encode_params" => {
                    params.libvpx_vp8 = encoder_libvpx_params::parse_vp8_encode_params(value)?;
                }
                "libvpx_vp9_encode_params" => {
                    params.libvpx_vp9 = encoder_libvpx_params::parse_vp9_encode_params(value)?;
                }
                "openh264_encode_params" => {
                    params.openh264 = encoder_openh264_params::parse_encode_params(value)?;
                }
                "svt_av1_encode_params" => {
                    params.svt_av1 = encoder_svt_av1_params::parse_encode_params(value)?;
                }
                #[cfg(target_os = "macos")]
                "video_toolbox_h264_encode_params" => {
                    params.video_toolbox_h264 =
                        encoder_video_toolbox_params::parse_h264_encode_params(value)?;
                }
                #[cfg(target_os = "macos")]
                "video_toolbox_h265_encode_params" => {
                    params.video_toolbox_h265 =
                        encoder_video_toolbox_params::parse_h265_encode_params(value)?;
                }
                _ => {}
            }
        }
        Ok(params)
    }
}

impl Default for LayoutEncodeParams {
    fn default() -> Self {
        todo!()
    }
}
