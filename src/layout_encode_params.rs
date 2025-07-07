#[derive(Debug, Clone)]
pub struct LayoutEncodeParams {
    // TODO: bitrate とかはこっちに移動してもいい

    // LibvpxVp8(shiguredo_libvpx::EncoderConfig),
    // LibvpxVp9(shiguredo_libvpx::EncoderConfig),
}

impl<'text> nojson::FromRawJsonValue<'text> for LayoutEncodeParams {
    fn from_raw_json_value(
        value: nojson::RawJsonValue<'text, '_>,
    ) -> Result<Self, nojson::JsonParseError> {
        todo!()
    }
}
