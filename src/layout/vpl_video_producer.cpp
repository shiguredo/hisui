#include "layout/vpl_video_producer.hpp"

#include <spdlog/spdlog.h>

#include <cstdint>
#include <vector>

#include <boost/rational.hpp>
#include <progresscpp/ProgressBar.hpp>

#include "config.hpp"
#include "layout/metadata.hpp"
#include "metadata.hpp"
#include "muxer/video_producer.hpp"
#include "video/basic_sequencer.hpp"
#include "video/composer.hpp"
#include "video/grid_composer.hpp"
#include "video/parallel_grid_composer.hpp"
#include "video/sequencer.hpp"
#include "video/vpl_encoder.hpp"

namespace hisui::layout {

VPLVideoProducer::VPLVideoProducer(const hisui::Config& t_config,
                                   const VPLVideoProducerParameters& params,
                                   const std::uint32_t t_fourcc)
    : VideoProducer({.show_progress_bar = t_config.show_progress_bar}),
      m_resolution(params.resolution) {
  m_frame_rate = t_config.out_video_frame_rate;
  m_duration = params.duration;

  hisui::video::VPLEncoderConfig vpl_config(m_resolution.width,
                                            m_resolution.height, t_config);

  for (auto& r : params.regions) {
    r->setEncodingInterval();
  }

  m_layout_composer = std::make_shared<Composer>(ComposerParameters{
      .regions = params.regions, .resolution = m_resolution});

  m_encoder = std::make_shared<hisui::video::VPLEncoder>(
      t_fourcc, &m_buffer, vpl_config, params.timescale);
}

void VPLVideoProducer::produce() {
  if (isFinished()) {
    return;
  }

  try {
    std::vector<unsigned char> raw_image;

    raw_image.resize(m_resolution.width * m_resolution.height * 3 >> 1);

    const std::uint64_t max_time = static_cast<std::uint64_t>(
        std::ceil(m_duration * hisui::Constants::NANO_SECOND));

    progresscpp::ProgressBar progress_bar(max_time, 60);

    for (std::uint64_t t = 0, step = hisui::Constants::NANO_SECOND *
                                     m_frame_rate.denominator() /
                                     m_frame_rate.numerator();
         t < max_time; t += step) {
      m_layout_composer->compose(&raw_image, t);
      {
        std::lock_guard<std::mutex> lock(m_mutex_buffer);
        m_encoder->outputImage(raw_image);
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
    throw;
  }
}

std::uint32_t VPLVideoProducer::getWidth() const {
  return m_resolution.width;
}

std::uint32_t VPLVideoProducer::getHeight() const {
  return m_resolution.height;
}

}  // namespace hisui::layout
