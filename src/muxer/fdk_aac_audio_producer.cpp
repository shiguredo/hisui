#include "muxer/fdk_aac_audio_producer.hpp"

#include <memory>

#include "audio/basic_sequencer.hpp"
#include "audio/buffer_fdk_aac_encoder.hpp"
#include "config.hpp"
#include "metadata.hpp"

namespace hisui::muxer {

FDKAACAudioProducer::FDKAACAudioProducer(
    const hisui::Config& t_config,
    const FDKAACAudioProducerParameters& params)
    : AudioProducer({.archives = params.archives,
                     .mixer = t_config.audio_mixer,
                     .duration = params.duration,
                     .show_progress_bar =
                         t_config.show_progress_bar && t_config.audio_only}) {
  m_encoder = std::make_shared<hisui::audio::BufferFDKAACEncoder>(
      &m_buffer, hisui::audio::BufferFDKAACEncoderParameters{
                     .bit_rate = t_config.out_aac_bit_rate});
}

}  // namespace hisui::muxer
