#include "muxer/vpx_video_producer.hpp"

#include <cstdint>

#include <boost/rational.hpp>

#include "config.hpp"
#include "metadata.hpp"
#include "muxer/video_producer.hpp"
#include "video/basic_sequencer.hpp"
#include "video/buffer_vpx_encoder.hpp"
#include "video/composer.hpp"
#include "video/grid_composer.hpp"
#include "video/parallel_grid_composer.hpp"
#include "video/sequencer.hpp"
#include "video/vpx.hpp"

namespace hisui::muxer {

VPXVideoProducer::VPXVideoProducer(const hisui::Config& t_config,
                                   const hisui::Metadata& t_metadata,
                                   const std::uint64_t timescale)
    : VideoProducer({.show_progress_bar = t_config.show_progress_bar}) {
  m_sequencer = new hisui::video::BasicSequencer(t_metadata.getArchives());

  const auto scaling_width = t_config.scaling_width != 0
                                 ? t_config.scaling_width
                                 : m_sequencer->getMaxWidth();
  const auto scaling_height = t_config.scaling_height != 0
                                  ? t_config.scaling_height
                                  : m_sequencer->getMaxHeight();

  switch (t_config.video_composer) {
    case hisui::config::VideoComposer::Grid:
      m_composer = new hisui::video::GridComposer(
          scaling_width, scaling_height, m_sequencer->getSize(),
          t_config.max_columns, t_config.video_scaler,
          t_config.libyuv_filter_mode);
      break;
    case hisui::config::VideoComposer::ParallelGrid:
      m_composer = new hisui::video::ParallelGridComposer(
          scaling_width, scaling_height, m_sequencer->getSize(),
          t_config.max_columns, t_config.video_scaler,
          t_config.libyuv_filter_mode);
      break;
  }

  hisui::video::VPXEncoderConfig vpx_config(m_composer->getWidth(),
                                            m_composer->getHeight(), t_config);

  m_encoder =
      new hisui::video::BufferVPXEncoder(&m_buffer, vpx_config, timescale);

  m_max_stop_time_offset = t_metadata.getMaxStopTimeOffset();
  m_frame_rate = t_config.out_video_frame_rate;
}

}  // namespace hisui::muxer
