#pragma once

#include <cstdint>
#include <vector>

#include "video/sequencer.hpp"

namespace hisui {

class Archive;

}

namespace hisui::video {

class YUVImage;

class BasicSequencer : public Sequencer {
 public:
  explicit BasicSequencer(const std::vector<hisui::Archive>&);
  ~BasicSequencer();

  SequencerGetYUVsResult getYUVs(std::vector<const YUVImage*>*,
                                 const std::uint64_t);

 private:
  const YUVImage* m_black_yuv_image;
};

}  // namespace hisui::video
