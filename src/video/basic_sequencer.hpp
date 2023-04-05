#pragma once

#include <cstdint>
#include <memory>
#include <vector>

#include "video/sequencer.hpp"

namespace hisui {

class ArchiveItem;

}

namespace hisui::video {

class YUVImage;

class BasicSequencer : public Sequencer {
 public:
  explicit BasicSequencer(const std::vector<hisui::ArchiveItem>&);

  SequencerGetYUVsResult getYUVs(std::vector<std::shared_ptr<YUVImage>>*,
                                 const std::uint64_t);

 private:
  std::shared_ptr<YUVImage> m_black_yuv_image;
};

}  // namespace hisui::video
