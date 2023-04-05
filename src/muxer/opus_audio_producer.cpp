#include "muxer/opus_audio_producer.hpp"

#include <opus_types.h>
#include <memory>

#include "audio/basic_sequencer.hpp"
#include "audio/buffer_opus_encoder.hpp"
#include "audio/mixer.hpp"
#include "config.hpp"
#include "metadata.hpp"
#include "muxer/audio_producer.hpp"

namespace hisui::muxer {

OpusAudioProducer::OpusAudioProducer(
    const hisui::Config& t_config,
    const std::vector<hisui::ArchiveItem> t_archives,
    const double t_duration,
    const std::uint64_t timescale)
    : AudioProducer({.archives = t_archives,
                     .mixer = t_config.audio_mixer,
                     .duration = t_duration,
                     .show_progress_bar =
                         t_config.show_progress_bar && t_config.audio_only}) {
  auto encoder = std::make_shared<hisui::audio::BufferOpusEncoder>(
      &m_buffer,
      hisui::audio::BufferOpusEncoderParameters{
          .bit_rate = t_config.out_opus_bit_rate, .timescale = timescale});
  m_skip = encoder->getSkip();
  m_encoder = encoder;
}

::opus_int32 OpusAudioProducer::getSkip() const {
  return m_skip;
}

}  // namespace hisui::muxer
