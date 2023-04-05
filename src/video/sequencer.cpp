#include "video/sequencer.hpp"

#include <bits/exception.h>
#include <spdlog/fmt/fmt.h>
#include <spdlog/spdlog.h>

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <filesystem>
#include <iterator>
#include <set>

#include "constants.hpp"
#include "metadata.hpp"
#include "util/interval.hpp"
#include "video/image_source.hpp"
#include "video/source.hpp"
#include "video/webm_source.hpp"

namespace hisui::video {

std::uint32_t Sequencer::getMaxWidth() const {
  return m_max_width;
}

std::uint32_t Sequencer::getMaxHeight() const {
  return m_max_height;
}

std::size_t Sequencer::getSize() const {
  return m_size;
}

MakeSequenceResult make_sequence(
    const std::vector<hisui::ArchiveItem>& archives) {
  MakeSequenceResult result;
  auto& sequence = result.sequence;
  auto& max_width = result.max_width;
  auto& max_height = result.max_height;

  const std::set<std::string> image_extensions({".png", ".jpg", ".jpeg"});

  for (const auto& archive : archives) {
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
    if (width > max_width) {
      max_width = width;
    }
    if (height > max_height) {
      max_height = height;
    }
    const auto connection_id = archive.getConnectionID();
    const auto it = std::find_if(
        std::begin(sequence), std::end(sequence),
        [connection_id](
            const std::pair<std::string,
                            std::shared_ptr<std::vector<SourceAndInterval>>>&
                elem) { return elem.first == connection_id; });
    std::shared_ptr<std::vector<SourceAndInterval>> v;
    if (it == std::end(sequence)) {
      v = std::make_shared<std::vector<SourceAndInterval>>();
      sequence.emplace_back(connection_id, v);
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

  return result;
}

}  // namespace hisui::video
