#include "muxer/fdk_aac_audio_producer.hpp"

#include "audio/basic_sequencer.hpp"
#include "audio/buffer_fdk_aac_encoder.hpp"
#include "audio/mixer.hpp"
#include "config.hpp"
#include "metadata.hpp"

namespace hisui::muxer {

FDKAACAudioProducer::FDKAACAudioProducer(
    const hisui::Config& t_config,
    const hisui::MetadataSet& t_metadata_set)
    : AudioProducer({.show_progress_bar =
                         t_config.show_progress_bar && t_config.audio_only}) {
  switch (t_config.audio_mixer) {
    case hisui::config::AudioMixer::Simple:
      m_mix_sample = hisui::audio::mix_sample_simple;
      break;
    case hisui::config::AudioMixer::Vttoth:
      m_mix_sample = hisui::audio::mix_sample_vttoth;
      break;
  }

  m_sequencer = new hisui::audio::BasicSequencer(t_metadata_set.getArchives());
  m_max_stop_time_offset = t_metadata_set.getMaxStopTimeOffset();

  m_encoder = new hisui::audio::BufferFDKAACEncoder(
      &m_buffer, {.bit_rate = t_config.out_aac_bit_rate});
}

}  // namespace hisui::muxer
