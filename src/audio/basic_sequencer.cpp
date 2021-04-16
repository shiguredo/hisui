#include "audio/basic_sequencer.hpp"

#include <bits/exception.h>
#include <spdlog/fmt/fmt.h>
#include <spdlog/spdlog.h>

#include <cmath>
#include <filesystem>

#include "audio/webm_source.hpp"
#include "constants.hpp"
#include "metadata.hpp"
#include "util/interval.hpp"

namespace hisui::audio {

BasicSequencer::BasicSequencer(const std::vector<hisui::Archive>& archives) {
  for (const auto& archive : archives) {
    const auto& path = archive.getPath();
    if (!(path.extension() == ".webm")) {
      spdlog::info("unsupported audio source: {}", path.string());
      continue;
    }
    m_sequence.push_back(
        {std::make_unique<hisui::audio::WebMSource>(path.string()),
         hisui::util::Interval(static_cast<std::uint64_t>(std::floor(
                                   archive.getStartTimeOffset() *
                                   hisui::Constants::PCM_SAMPLE_RATE)),
                               static_cast<std::uint64_t>(std::ceil(
                                   archive.getStopTimeOffset() *
                                   hisui::Constants::PCM_SAMPLE_RATE)))});
  }
}

void BasicSequencer::getSamples(
    std::vector<std::pair<std::int16_t, std::int16_t>>* samples,
    const std::uint64_t position) {
  samples->clear();
  for (const auto& s : m_sequence) {
    if (s.second.isIn(position)) {
      samples->push_back(
          s.first->getSample(s.second.getSubstructLower(position)));
    }
  }
}

}  // namespace hisui::audio
