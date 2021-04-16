#include "muxer/multi_channel_vpx_video_producer.hpp"

#include <bits/exception.h>
#include <spdlog/fmt/fmt.h>
#include <spdlog/spdlog.h>

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <mutex>
#include <vector>

#include <boost/rational.hpp>
#include <progresscpp/ProgressBar.hpp>

#include "config.hpp"
#include "metadata.hpp"
#include "muxer/video_producer.hpp"
#include "video/buffer_vpx_encoder.hpp"
#include "video/composer.hpp"
#include "video/encoder.hpp"
#include "video/grid_composer.hpp"
#include "video/multi_channel_sequencer.hpp"
#include "video/parallel_grid_composer.hpp"
#include "video/sequencer.hpp"
#include "video/vpx.hpp"

namespace hisui::video {

class YUVImage;

}

namespace hisui::muxer {

MultiChannelVPXVideoProducer::MultiChannelVPXVideoProducer(
    const hisui::Config& t_config,
    const hisui::MetadataSet& t_metadata_set,
    const std::uint64_t timescale)
    : VideoProducer({.show_progress_bar = t_config.show_progress_bar}),
      m_normal_bit_rate(t_config.out_video_bit_rate),
      m_preferred_bit_rate(t_config.screen_capture_bit_rate) {
  m_sequencer = new hisui::video::MultiChannelSequencer(
      t_metadata_set.getNormal().getArchives(),
      t_metadata_set.getPreferred().getArchives());

  const auto scaling_width = t_config.scaling_width != 0
                                 ? t_config.scaling_width
                                 : m_sequencer->getMaxWidth();
  const auto scaling_height = t_config.scaling_height != 0
                                  ? t_config.scaling_height
                                  : m_sequencer->getMaxHeight();

  switch (t_config.video_composer) {
    case hisui::config::VideoComposer::Grid:
      m_normal_channel_composer = new hisui::video::GridComposer(
          scaling_width, scaling_height, m_sequencer->getSize(),
          t_config.max_columns, t_config.video_scaler,
          t_config.libyuv_filter_mode);
      m_preferred_channel_composer = new hisui::video::GridComposer(
          t_config.screen_capture_width, t_config.screen_capture_height, 1, 1,
          t_config.video_scaler, t_config.libyuv_filter_mode);
      break;
    case hisui::config::VideoComposer::ParallelGrid:
      m_normal_channel_composer = new hisui::video::ParallelGridComposer(
          scaling_width, scaling_height, m_sequencer->getSize(),
          t_config.max_columns, t_config.video_scaler,
          t_config.libyuv_filter_mode);
      m_preferred_channel_composer = new hisui::video::GridComposer(
          t_config.screen_capture_width, t_config.screen_capture_height, 1, 1,
          t_config.video_scaler, t_config.libyuv_filter_mode);
      break;
  }

  m_composer = m_normal_channel_composer;

  hisui::video::VPXEncoderConfig vpx_config(
      std::max(m_normal_channel_composer->getWidth(),
               m_preferred_channel_composer->getWidth()),
      std::max(m_normal_channel_composer->getHeight(),
               m_preferred_channel_composer->getHeight()),
      t_config);

  m_encoder =
      new hisui::video::BufferVPXEncoder(&m_buffer, vpx_config, timescale);

  m_max_stop_time_offset = t_metadata_set.getMaxStopTimeOffset();
  m_frame_rate = t_config.out_video_frame_rate;
}

MultiChannelVPXVideoProducer::~MultiChannelVPXVideoProducer() {
  delete m_normal_channel_composer;
  m_normal_channel_composer = nullptr;
  delete m_preferred_channel_composer;
  m_preferred_channel_composer = nullptr;
  m_composer = nullptr;
}

void MultiChannelVPXVideoProducer::produce() {
  if (isFinished()) {
    return;
  }

  try {
    std::vector<const video::YUVImage*> yuvs;
    std::vector<unsigned char> raw_image;
    yuvs.resize(m_sequencer->getSize());

    const std::uint64_t max_time = static_cast<std::uint64_t>(
        std::ceil(m_max_stop_time_offset * hisui::Constants::NANO_SECOND));

    progresscpp::ProgressBar progress_bar(max_time, 60);

    for (std::uint64_t t = 0, step = hisui::Constants::NANO_SECOND *
                                     m_frame_rate.denominator() /
                                     m_frame_rate.numerator();
         t < max_time; t += step) {
      auto result = m_sequencer->getYUVs(&yuvs, t);
      if (result.is_preferred_stream) {
        raw_image.resize(m_preferred_channel_composer->getWidth() *
                             m_preferred_channel_composer->getHeight() * 3 >>
                         1);
        m_preferred_channel_composer->compose(&raw_image, {yuvs[0]});
        m_encoder->setResolutionAndBitrate(
            m_preferred_channel_composer->getWidth(),
            m_preferred_channel_composer->getHeight(), m_preferred_bit_rate);
        {
          std::lock_guard<std::mutex> lock(m_mutex_buffer);
          m_encoder->outputImage(raw_image);
        }

      } else {
        raw_image.resize(m_normal_channel_composer->getWidth() *
                             m_normal_channel_composer->getHeight() * 3 >>
                         1);
        m_normal_channel_composer->compose(&raw_image, yuvs);
        m_encoder->setResolutionAndBitrate(
            m_normal_channel_composer->getWidth(),
            m_normal_channel_composer->getHeight(), m_normal_bit_rate);
        {
          std::lock_guard<std::mutex> lock(m_mutex_buffer);
          m_encoder->outputImage(raw_image);
        }
      }
      if (m_show_progress_bar) {
        progress_bar.setTicks(t);
        progress_bar.display();
      }
    }

    {
      std::lock_guard<std::mutex> lock(m_mutex_buffer);
      m_encoder->flush();
      m_is_finished = true;
    }

    if (m_show_progress_bar) {
      progress_bar.setTicks(max_time);
      progress_bar.done();
    }
  } catch (const std::exception& e) {
    spdlog::error("VideoProducer::produce() failed: what={}", e.what());
    m_is_finished = true;
    throw e;
  }
}

}  // namespace hisui::muxer
