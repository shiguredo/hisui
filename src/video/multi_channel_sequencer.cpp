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
  m_max_width = 0;
  m_max_height = 0;

  const std::set<std::string> image_extensions({".png", ".jpg", ".jpeg"});

  for (const auto& archive : original_archives) {
    Source* source;
    const auto& path = archive.getPath();
    const auto extension = path.extension();
    if (extension == ".webm") {
      source = new WebMSource(path.string());
    } else if (image_extensions.contains(extension)) {
      source = new ImageSource(path.string());
    } else {
      spdlog::info("unsupported video source: {}", path.string());
      continue;
    }
    const auto width = source->getWidth();
    const auto height = source->getHeight();
    if (width > m_max_width) {
      m_max_width = width;
    }
    if (height > m_max_height) {
      m_max_height = height;
    }
    const auto connection_id = archive.getConnectionID();
    const auto it = std::find_if(
        std::begin(m_sequence), std::end(m_sequence),
        [connection_id](
            const std::pair<std::string,
                            std::shared_ptr<std::vector<SourceAndInterval>>>&
                elem) { return elem.first == connection_id; });
    std::shared_ptr<std::vector<SourceAndInterval>> v;
    if (it == std::end(m_sequence)) {
      v = std::make_shared<std::vector<SourceAndInterval>>();
      m_sequence.emplace_back(connection_id, v);
    } else {
      v = (*it).second;
    }
    v->push_back({std::unique_ptr<Source>(source),
                  hisui::util::Interval(static_cast<std::uint64_t>(std::floor(
                                            archive.getStartTimeOffset() *
                                            hisui::Constants::NANO_SECOND)),
                                        static_cast<std::uint64_t>(std::ceil(
                                            archive.getStopTimeOffset() *
                                            hisui::Constants::NANO_SECOND)))});
  }

  m_size = std::size(m_sequence);

  // codec には奇数をあたえるとおかしな動作をするものがあるので, 4の倍数に切り上げる
  m_max_width = ((m_max_width + 3) >> 2) << 2;
  m_max_height = ((m_max_height + 3) >> 2) << 2;

  spdlog::debug("m_max_width x m_max_height: {} x {}", m_max_width,
                m_max_height);

  m_black_yuv_image = create_black_yuv_image(m_max_width, m_max_height);

  for (const auto& archive : alternative_archives) {
    Source* source;
    const auto& path = archive.getPath();
    const auto extension = path.extension();
    if (extension == ".webm") {
      source = new WebMSource(path.string());
    } else if (image_extensions.contains(extension)) {
      source = new ImageSource(path.string());
    } else {
      spdlog::info("unsupported video source: {}", path.string());
      continue;
    }
    const auto connection_id = archive.getConnectionID();
    const auto it = std::find_if(
        std::begin(m_alternative_sequence), std::end(m_alternative_sequence),
        [connection_id](
            const std::pair<std::string,
                            std::shared_ptr<std::vector<SourceAndInterval>>>&
                elem) { return elem.first == connection_id; });
    std::shared_ptr<std::vector<SourceAndInterval>> v;
    if (it == std::end(m_alternative_sequence)) {
      v = std::make_shared<std::vector<SourceAndInterval>>();
      m_alternative_sequence.emplace_back(connection_id, v);
    } else {
      v = (*it).second;
    }
    v->push_back({std::unique_ptr<Source>(source),
                  hisui::util::Interval(static_cast<std::uint64_t>(std::floor(
                                            archive.getStartTimeOffset() *
                                            hisui::Constants::NANO_SECOND)),
                                        static_cast<std::uint64_t>(std::ceil(
                                            archive.getStopTimeOffset() *
                                            hisui::Constants::NANO_SECOND)))});
  }
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
