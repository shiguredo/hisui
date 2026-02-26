use crate::audio::{Channels, SampleRate};

use shiguredo_mp4::{
    FixedPointNumber, Uint,
    boxes::{AudioSampleEntryFields, EsdsBox, Mp4aBox, SampleEntry},
    descriptors::{DecoderConfigDescriptor, DecoderSpecificInfo, EsDescriptor},
};

pub fn parse_audio_specific_config(data: &[u8]) -> crate::Result<(SampleRate, Channels)> {
    if data.len() < 2 {
        return Err(crate::Error::new("AAC audio specific config is too short"));
    }

    let byte0 = data[0];
    let byte1 = data[1];
    let sample_rate_index = ((byte0 & 0x07) << 1) | (byte1 >> 7);
    let channel_configuration = (byte1 >> 3) & 0x0F;

    let sample_rate = sample_rate_from_sampling_frequency_index(sample_rate_index)?;
    let channels = Channels::from_u8(channel_configuration)?;
    Ok((sample_rate, channels))
}

pub fn sample_rate_from_sampling_frequency_index(index: u8) -> crate::Result<SampleRate> {
    let sample_rate = match index {
        0 => 96_000,
        1 => 88_200,
        2 => 64_000,
        3 => 48_000,
        4 => 44_100,
        5 => 32_000,
        6 => 24_000,
        7 => 22_050,
        8 => 16_000,
        9 => 12_000,
        10 => 11_025,
        11 => 8_000,
        12 => 7_350,
        _ => return Err(crate::Error::new("invalid AAC sample rate index")),
    };
    SampleRate::from_u32(sample_rate)
}

pub fn create_audio_specific_config(
    audio_object_type: u8,
    sampling_frequency_index: u8,
    channel_configuration: u8,
) -> Vec<u8> {
    let byte0 = (audio_object_type << 3) | ((sampling_frequency_index >> 1) & 0x07);
    let byte1 = ((sampling_frequency_index & 0x01) << 7) | ((channel_configuration & 0x0F) << 3);
    vec![byte0, byte1]
}

pub fn create_mp4a_sample_entry(
    audio_specific_config: &[u8],
    sample_rate: SampleRate,
    channels: Channels,
) -> crate::Result<SampleEntry> {
    let sample_rate_u16 = sample_rate.as_u16()?;

    Ok(SampleEntry::Mp4a(Mp4aBox {
        audio: AudioSampleEntryFields {
            data_reference_index: AudioSampleEntryFields::DEFAULT_DATA_REFERENCE_INDEX,
            channelcount: u16::from(channels.get()),
            samplesize: 16,
            samplerate: FixedPointNumber::new(sample_rate_u16, 0),
        },
        esds_box: EsdsBox {
            es: EsDescriptor {
                es_id: EsDescriptor::MIN_ES_ID,
                stream_priority: EsDescriptor::LOWEST_STREAM_PRIORITY,
                depends_on_es_id: None,
                url_string: None,
                ocr_es_id: None,
                dec_config_descr: DecoderConfigDescriptor {
                    object_type_indication:
                        DecoderConfigDescriptor::OBJECT_TYPE_INDICATION_AUDIO_ISO_IEC_14496_3,
                    stream_type: DecoderConfigDescriptor::STREAM_TYPE_AUDIO,
                    up_stream: DecoderConfigDescriptor::UP_STREAM_FALSE,
                    dec_specific_info: Some(DecoderSpecificInfo {
                        payload: audio_specific_config.to_vec(),
                    }),
                    buffer_size_db: Uint::new(65536),
                    max_bitrate: 256000,
                    avg_bitrate: 128000,
                },
                sl_config_descr: shiguredo_mp4::descriptors::SlConfigDescriptor,
            },
        },
        unknown_boxes: Vec::new(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_audio_specific_config_parses_basic_values() {
        let (sample_rate, channels) =
            parse_audio_specific_config(&[0x12, 0x10]).expect("must parse config");
        assert_eq!(sample_rate.get(), 44_100);
        assert_eq!(channels.get(), 2);
    }

    #[test]
    fn create_mp4a_sample_entry_keeps_audio_specific_config() {
        let sample_entry = create_mp4a_sample_entry(
            &[0x12, 0x10],
            SampleRate::from_u32(44_100).expect("must create sample rate"),
            Channels::STEREO,
        )
        .expect("must create sample entry");

        let SampleEntry::Mp4a(mp4a) = sample_entry else {
            panic!("expected Mp4a sample entry");
        };

        assert_eq!(mp4a.audio.channelcount, 2);
        assert_eq!(mp4a.audio.samplerate.integer, 44_100);
        assert_eq!(
            mp4a.esds_box
                .es
                .dec_config_descr
                .dec_specific_info
                .as_ref()
                .expect("AudioSpecificConfig must exist")
                .payload,
            vec![0x12, 0x10]
        );
    }
}
