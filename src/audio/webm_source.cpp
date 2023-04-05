#include "audio/webm_source.hpp"

#include <bits/exception.h>
#include <spdlog/fmt/fmt.h>
#include <spdlog/spdlog.h>

#include <cstddef>
#include <iterator>

#include "audio/decoder.hpp"
#include "audio/opus_decoder.hpp"
#include "constants.hpp"
#include "report/reporter.hpp"
#include "webm/input/audio_context.hpp"

namespace hisui::audio {

WebMSource::WebMSource(const std::string& t_file_path) {
  m_webm = std::make_shared<hisui::webm::input::AudioContext>(t_file_path);
  if (!m_webm->init()) {
    spdlog::info(
        "AudioContext initialization failed. no audio track or unsupported "
        "codec: file_path='{}'",
        t_file_path);
    m_webm = nullptr;
    if (hisui::report::Reporter::hasInstance()) {
      hisui::report::Reporter::getInstance().registerAudioDecoder(
          t_file_path, {.codec = "none", .channels = 0, .duration = 0});
    }
    return;
  }

  switch (m_webm->getCodec()) {
    case hisui::webm::input::AudioCodec::Opus:
      m_channels = m_webm->getChannels(),
      m_sampling_rate = static_cast<std::uint64_t>(m_webm->getSamplingRate());
      m_decoder = std::make_shared<OpusDecoder>(m_channels);
      if (hisui::report::Reporter::hasInstance()) {
        hisui::report::Reporter::getInstance().registerAudioDecoder(
            m_webm->getFilePath(), {.codec = "opus",
                                    .channels = m_webm->getChannels(),
                                    .duration = m_webm->getDuration()});
      }
      break;
    default:
      // 対応していない WebM の場合は {0, 0} を返す
      m_webm = nullptr;
      spdlog::info("unsupported audio codec: file_path ='{}'", t_file_path);
      if (hisui::report::Reporter::hasInstance()) {
        hisui::report::Reporter::getInstance().registerAudioDecoder(
            t_file_path,
            {.codec = "unsupported", .channels = 0, .duration = 0});
      }
      return;
  }
}

std::pair<std::int16_t, std::int16_t> WebMSource::getSample(
    const std::uint64_t position) {
  if (!m_decoder) {
    return {0, 0};
  }
  if (position < m_current_position) {
    return {0, 0};
  }

  if (std::empty(m_data)) {
    // データが空だったら次のフレームを読んで, その後の m_decoder と m_current_position の値に応じた処理を行なう
    readFrame();
    return getSample(position);
  }

  if (m_channels == 1) {
    const std::int16_t d = m_data.front();
    m_data.pop();
    return {d, d};
  }

  const std::int16_t f = m_data.front();
  m_data.pop();
  const std::int16_t s = m_data.front();
  m_data.pop();
  return {f, s};
}

void WebMSource::readFrame() {
  if (m_webm->readFrame()) {
    m_current_position = static_cast<std::uint64_t>(m_webm->getTimestamp()) *
                         m_sampling_rate / hisui::Constants::NANO_SECOND;
    const auto decoded =
        m_decoder->decode(m_webm->getBuffer(), m_webm->getBufferSize());
    if (decoded.second > 0) {
      for (std::size_t i = 0; i < decoded.second; ++i) {
        m_data.push(decoded.first[i]);
      }
    }
  } else {
    m_decoder = nullptr;
  }
}

}  // namespace hisui::audio
