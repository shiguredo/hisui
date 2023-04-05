#pragma once

#include <cstdint>
#include <memory>
#include <string>

#include "video/source.hpp"

namespace hisui::video {

class YUVImage;

class ImageSource : public Source {
 public:
  explicit ImageSource(const std::string&);
  const std::shared_ptr<YUVImage> getYUV(const std::uint64_t);
  std::uint32_t getWidth() const;
  std::uint32_t getHeight() const;

 private:
  std::uint32_t m_width;
  std::uint32_t m_height;
  std::shared_ptr<YUVImage> m_yuv_image;
};

}  // namespace hisui::video
