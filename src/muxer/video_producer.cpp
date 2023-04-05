#include "muxer/video_producer.hpp"

#include <bits/exception.h>
#include <spdlog/fmt/fmt.h>
#include <spdlog/spdlog.h>

#include <cmath>
#include <cstdint>
#include <memory>
#include <mutex>
#include <optional>
#include <vector>

#include <boost/rational.hpp>
#include <progresscpp/ProgressBar.hpp>

#include "constants.hpp"
#include "frame.hpp"
#include "video/composer.hpp"
#include "video/encoder.hpp"
#include "video/sequencer.hpp"

namespace hisui::video {

class YUVImage;

}

namespace hisui::muxer {

VideoProducer::VideoProducer(const VideoProducerParameters& params)
    : m_show_progress_bar(params.show_progress_bar),
      m_is_finished(params.is_finished) {}

void VideoProducer::produce() {
  if (isFinished()) {
    return;
  }

  try {
    std::vector<std::shared_ptr<video::YUVImage>> yuvs;
    std::vector<unsigned char> raw_image;
    yuvs.resize(m_sequencer->getSize());
    raw_image.resize(m_composer->getWidth() * m_composer->getHeight() * 3 >> 1);

    const std::uint64_t max_time = static_cast<std::uint64_t>(
        std::ceil(m_duration * hisui::Constants::NANO_SECOND));

    progresscpp::ProgressBar progress_bar(max_time, 60);

    for (std::uint64_t t = 0, step = hisui::Constants::NANO_SECOND *
                                     m_frame_rate.denominator() /
                                     m_frame_rate.numerator();
         t < max_time; t += step) {
      m_sequencer->getYUVs(&yuvs, t);
      m_composer->compose(&raw_image, yuvs);
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

void VideoProducer::bufferPop() {
  std::lock_guard<std::mutex> lock(m_mutex_buffer);
  m_buffer.pop();
}

std::optional<hisui::Frame> VideoProducer::bufferFront() {
  std::lock_guard<std::mutex> lock(m_mutex_buffer);
  if (m_buffer.empty()) {
    return {};
  }
  return m_buffer.front();
}

bool VideoProducer::isFinished() {
  std::lock_guard<std::mutex> lock(m_mutex_buffer);
  return m_is_finished && m_buffer.empty();
}

std::uint32_t VideoProducer::getWidth() const {
  return m_composer->getWidth();
}

std::uint32_t VideoProducer::getHeight() const {
  return m_composer->getHeight();
}

std::uint32_t VideoProducer::getFourcc() const {
  return m_encoder->getFourcc();
}

}  // namespace hisui::muxer
