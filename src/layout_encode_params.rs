#[cfg(feature = "nvcodec")]
use crate::encoder_nvcodec_params;
#[cfg(target_os = "macos")]
use crate::encoder_video_toolbox_params;
use crate::layout::DEFAULT_LAYOUT_JSON;
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
    #[cfg(feature = "nvcodec")]
    pub nvcodec_h264: shiguredo_nvcodec::EncoderConfig,
    #[cfg(feature = "nvcodec")]
    pub nvcodec_h265: shiguredo_nvcodec::EncoderConfig,
    #[cfg(feature = "nvcodec")]
    pub nvcodec_av1: shiguredo_nvcodec::EncoderConfig,
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
                #[cfg(feature = "nvcodec")]
                "nvcodec_h264_encode_params" => {
                    params.nvcodec_h264 = encoder_nvcodec_params::parse_h264_encode_params(value)?;
                }
                #[cfg(feature = "nvcodec")]
                "nvcodec_h265_encode_params" => {
                    params.nvcodec_h265 = encoder_nvcodec_params::parse_h265_encode_params(value)?;
                }
                #[cfg(feature = "nvcodec")]
                "nvcodec_av1_encode_params" => {
                    params.nvcodec_av1 = encoder_nvcodec_params::parse_av1_encode_params(value)?;
                }
                _ => {}
            }
        }
        Ok(params)
    }
}

impl LayoutEncodeParams {
    fn new_from_default_layout() -> Result<Self, nojson::JsonParseError> {
        let default_layout = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
        let value = default_layout.value();

        let libvpx_vp8 = encoder_libvpx_params::parse_vp8_encode_params(
            value.to_member("libvpx_vp8_encode_params")?.required()?,
        )?;

        let libvpx_vp9 = encoder_libvpx_params::parse_vp9_encode_params(
            value.to_member("libvpx_vp9_encode_params")?.required()?,
        )?;

        let openh264 = encoder_openh264_params::parse_encode_params(
            value.to_member("openh264_encode_params")?.required()?,
        )?;

        let svt_av1 = encoder_svt_av1_params::parse_encode_params(
            value.to_member("svt_av1_encode_params")?.required()?,
        )?;

        #[cfg(target_os = "macos")]
        let video_toolbox_h264 = encoder_video_toolbox_params::parse_h264_encode_params(
            value
                .to_member("video_toolbox_h264_encode_params")?
                .required()?,
        )?;

        #[cfg(target_os = "macos")]
        let video_toolbox_h265 = encoder_video_toolbox_params::parse_h265_encode_params(
            value
                .to_member("video_toolbox_h265_encode_params")?
                .required()?,
        )?;

        #[cfg(feature = "nvcodec")]
        let nvcodec_h264 = encoder_nvcodec_params::parse_h264_encode_params(
            value.to_member("nvcodec_h264_encode_params")?.required()?,
        )?;

        #[cfg(feature = "nvcodec")]
        let nvcodec_h265 = encoder_nvcodec_params::parse_h265_encode_params(
            value.to_member("nvcodec_h265_encode_params")?.required()?,
        )?;

        #[cfg(feature = "nvcodec")]
        let nvcodec_av1 = encoder_nvcodec_params::parse_av1_encode_params(
            value.to_member("nvcodec_av1_encode_params")?.required()?,
        )?;

        Ok(Self {
            libvpx_vp8,
            libvpx_vp9,
            openh264,
            svt_av1,
            #[cfg(target_os = "macos")]
            video_toolbox_h264,
            #[cfg(target_os = "macos")]
            video_toolbox_h265,
            #[cfg(feature = "nvcodec")]
            nvcodec_h264,
            #[cfg(feature = "nvcodec")]
            nvcodec_h265,
            #[cfg(feature = "nvcodec")]
            nvcodec_av1,
        })
    }
}

impl Default for LayoutEncodeParams {
    fn default() -> Self {
        Self::new_from_default_layout().expect("bug")
    }
}
