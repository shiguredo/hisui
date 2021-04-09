#include "video/multi_channel_sequencer.hpp"

#include <bits/exception.h>
#include <spdlog/spdlog.h>

#include <algorithm>
#include <cmath>
#include <cstddef>
#include <filesystem>
#include <iterator>
#include <memory>
#include <set>
#include <string>
#include <utility>

#include "constants.hpp"
#include "metadata.hpp"
#include "util/interval.hpp"
#include "video/image_source.hpp"
#include "video/sequencer.hpp"
#include "video/source.hpp"
#include "video/webm_source.hpp"
#include "video/yuv.hpp"

namespace hisui::video {

MultiChannelSequencer::MultiChannelSequencer(
    const std::vector<hisui::Archive>& original_archives,
    const std::vector<hisui::Archive>& alternative_archives) {
  auto original_result = make_sequence(original_archives);

  m_sequence = original_result.sequence;
  m_size = std::size(m_sequence);

  m_max_width = ((original_result.max_width + 3) >> 2) << 2;
  m_max_height = ((original_result.max_height + 3) >> 2) << 2;

  spdlog::debug("m_max_width x m_max_height: {} x {}", m_max_width,
                m_max_height);

  m_black_yuv_image = create_black_yuv_image(m_max_width, m_max_height);

  auto alternative_result = make_sequence(alternative_archives);

  m_alternative_sequence = alternative_result.sequence;
}  // namespace hisui::video

MultiChannelSequencer::~MultiChannelSequencer() {
  delete m_black_yuv_image;
}

SequencerGetYUVsResult MultiChannelSequencer::getYUVs(
    std::vector<const YUVImage*>* yuvs,
    const std::uint64_t timestamp) {
  for (const auto& p : m_alternative_sequence) {
    const auto it = std::find_if(
        std::begin(*p.second), std::end(*p.second),
        [timestamp](const auto& s) { return s.second.isIn(timestamp); });
    if (it != std::end(*p.second)) {
      spdlog::debug("alternative");
      (*yuvs)[0] = it->first->getYUV(it->second.getSubstructLower(timestamp));
      return {.is_alternative_stream = true};
    }
  }

  spdlog::debug("original");
  std::size_t i = 0;
  for (const auto& p : m_sequence) {
    const auto it = std::find_if(
        std::begin(*p.second), std::end(*p.second),
        [timestamp](const auto& s) { return s.second.isIn(timestamp); });
    if (it == std::end(*p.second)) {
      (*yuvs)[i] = m_black_yuv_image;
    } else {
      (*yuvs)[i] = it->first->getYUV(it->second.getSubstructLower(timestamp));
    }
    ++i;
  }
  return {.is_alternative_stream = false};
}

}  // namespace hisui::video
