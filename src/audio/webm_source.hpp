#pragma once

#include <cstdint>
#include <memory>
#include <queue>
#include <string>
#include <utility>

#include "audio/source.hpp"

namespace hisui::webm::input {

class AudioContext;

}

namespace hisui::audio {

class Decoder;

class WebMSource : public Source {
 public:
  explicit WebMSource(const std::string&);
  std::pair<std::int16_t, std::int16_t> getSample(const std::uint64_t);

 private:
  std::shared_ptr<hisui::webm::input::AudioContext> m_webm = nullptr;
  std::shared_ptr<hisui::audio::Decoder> m_decoder = nullptr;
  int m_channels;
  std::uint64_t m_sampling_rate;
  std::queue<std::int16_t> m_data;
  std::uint64_t m_current_position = 0;

  void readFrame();
};

}  // namespace hisui::audio
