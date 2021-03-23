#include "muxer/audio_producer.hpp"

#include <bits/exception.h>
#include <spdlog/spdlog.h>

#include <cmath>
#include <mutex>
#include <utility>
#include <vector>

#include "audio/encoder.hpp"
#include "audio/sequencer.hpp"
#include "constants.hpp"

namespace hisui::muxer {

AudioProducer::~AudioProducer() {
  if (m_sequencer) {
    delete m_sequencer;
  }
  if (m_encoder) {
    delete m_encoder;
  }
}

void AudioProducer::produce() {
  try {
    std::vector<std::pair<std::int16_t, std::int16_t>> samples;

    for (std::uint64_t p = 0, m = static_cast<std::uint64_t>(
                                  std::ceil(m_max_stop_time_offset *
                                            hisui::Constants::PCM_SAMPLE_RATE));
         p < m; ++p) {
      std::int16_t left = 0;
      std::int16_t right = 0;
      m_sequencer->getSamples(&samples, p);
      for (const auto& s : samples) {
        const auto [l, r] = s;
        if (l != 0) {
          left = m_mix_sample(left, l);
        }
        if (r != 0) {
          right = m_mix_sample(right, r);
        }
      }
      {
        std::lock_guard<std::mutex> lock(m_mutex_buffer);
        m_encoder->addSample(left, right);
      }
    }

    {
      std::lock_guard<std::mutex> lock(m_mutex_buffer);
      m_encoder->flush();
      m_is_finished = true;
    }
  } catch (const std::exception& e) {
    spdlog::error("AudioProducer::produce() failed: what={}", e.what());
    m_is_finished = true;
    throw e;
  }
}

void AudioProducer::bufferPop() {
  std::lock_guard<std::mutex> lock(m_mutex_buffer);
  m_buffer.pop();
}

std::optional<hisui::Frame> AudioProducer::bufferFront() {
  std::lock_guard<std::mutex> lock(m_mutex_buffer);
  if (m_buffer.empty()) {
    return {};
  }
  return m_buffer.front();
}

bool AudioProducer::isFinished() {
  std::lock_guard<std::mutex> lock(m_mutex_buffer);
  return m_is_finished && m_buffer.empty();
}

}  // namespace hisui::muxer
