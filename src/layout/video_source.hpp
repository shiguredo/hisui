#pragma once

#include <cstdint>
#include <memory>

#include "layout/source.hpp"

namespace hisui::video {

class Source;
class YUVImage;

}  // namespace hisui::video

namespace hisui::layout {
class VideoSource : public Source {
 public:
  explicit VideoSource(const SourceParameters&);
  const std::shared_ptr<hisui::video::YUVImage> getYUV(const std::uint64_t);

 private:
  std::shared_ptr<hisui::video::Source> m_source;
};

}  // namespace hisui::layout
