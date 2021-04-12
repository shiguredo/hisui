#pragma once

#include <cstdint>
#include <memory>
#include <string>
#include <utility>
#include <vector>

#include "video/sequencer.hpp"

namespace hisui {

class Archive;

}

namespace hisui::video {

class YUVImage;

class MultiChannelSequencer : public Sequencer {
 public:
  explicit MultiChannelSequencer(const std::vector<hisui::Archive>&,
                                 const std::vector<hisui::Archive>&);
  ~MultiChannelSequencer();

  SequencerGetYUVsResult getYUVs(std::vector<const YUVImage*>*,
                                 const std::uint64_t);

 private:
  const YUVImage* m_black_yuv_image;
  std::vector<
      std::pair<std::string, std::shared_ptr<std::vector<SourceAndInterval>>>>
      m_preferred_sequence;
};

}  // namespace hisui::video
