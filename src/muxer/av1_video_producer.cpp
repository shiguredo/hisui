#include "muxer/av1_video_producer.hpp"

#include <cstdint>
#include <memory>

#include <boost/rational.hpp>

#include "config.hpp"
#include "metadata.hpp"
#include "muxer/video_producer.hpp"
#include "video/basic_sequencer.hpp"
#include "video/buffer_av1_encoder.hpp"
#include "video/composer.hpp"
#include "video/grid_composer.hpp"
#include "video/parallel_grid_composer.hpp"
#include "video/sequencer.hpp"

namespace hisui::muxer {

AV1VideoProducer::AV1VideoProducer(const hisui::Config& t_config,
                                   const AV1VideoProducerParameters& params)
    : VideoProducer({.show_progress_bar = t_config.show_progress_bar}) {
  m_sequencer = std::make_shared<hisui::video::BasicSequencer>(params.archives);

  const auto scaling_width = t_config.scaling_width != 0
                                 ? t_config.scaling_width
                                 : m_sequencer->getMaxWidth();
  const auto scaling_height = t_config.scaling_height != 0
                                  ? t_config.scaling_height
                                  : m_sequencer->getMaxHeight();

  switch (t_config.video_composer) {
    case hisui::config::VideoComposer::Grid:
      m_composer = std::make_shared<hisui::video::GridComposer>(
          scaling_width, scaling_height, m_sequencer->getSize(),
          t_config.max_columns, t_config.video_scaler,
          t_config.libyuv_filter_mode);
      break;
    case hisui::config::VideoComposer::ParallelGrid:
      m_composer = std::make_shared<hisui::video::ParallelGridComposer>(
          scaling_width, scaling_height, m_sequencer->getSize(),
          t_config.max_columns, t_config.video_scaler,
          t_config.libyuv_filter_mode);
      break;
  }

  hisui::video::AV1EncoderConfig av1_config(m_composer->getWidth(),
                                            m_composer->getHeight(), t_config);

  m_encoder = std::make_shared<hisui::video::BufferAV1Encoder>(
      &m_buffer, av1_config, params.timescale);

  m_duration = params.duration;
  m_frame_rate = t_config.out_video_frame_rate;
}

const std::vector<std::uint8_t>& AV1VideoProducer::getExtraData() const {
  return m_encoder->getExtraData();
}

}  // namespace hisui::muxer
