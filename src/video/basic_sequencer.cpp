#include "video/basic_sequencer.hpp"

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

BasicSequencer::BasicSequencer(const std::vector<hisui::Archive>& archives) {
  auto result = make_sequence(archives);

  m_sequence = result.sequence;
  m_size = std::size(m_sequence);

  // codec には奇数をあたえるとおかしな動作をするものがあるので, 4の倍数に切り上げる
  m_max_width = ((result.max_width + 3) >> 2) << 2;
  m_max_height = ((result.max_height + 3) >> 2) << 2;

  spdlog::debug("m_max_width x m_max_height: {} x {}", m_max_width,
                m_max_height);

  m_black_yuv_image = create_black_yuv_image(m_max_width, m_max_height);
}  // namespace hisui::video

BasicSequencer::~BasicSequencer() {
  delete m_black_yuv_image;
}

SequencerGetYUVsResult BasicSequencer::getYUVs(
    std::vector<const YUVImage*>* yuvs,
    const std::uint64_t timestamp) {
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
  return {};
}

}  // namespace hisui::video
