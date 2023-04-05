#pragma once

#include <cstdint>
#include <memory>
#include <string>
#include <utility>
#include <vector>

#include "video/sequencer.hpp"

namespace hisui {

class ArchiveItem;

}

namespace hisui::video {

class YUVImage;

class MultiChannelSequencer : public Sequencer {
 public:
  explicit MultiChannelSequencer(const std::vector<hisui::ArchiveItem>&,
                                 const std::vector<hisui::ArchiveItem>&);

  SequencerGetYUVsResult getYUVs(std::vector<std::shared_ptr<YUVImage>>*,
                                 const std::uint64_t);

 private:
  std::shared_ptr<YUVImage> m_black_yuv_image;
  std::vector<
      std::pair<std::string, std::shared_ptr<std::vector<SourceAndInterval>>>>
      m_preferred_sequence;
};

}  // namespace hisui::video
