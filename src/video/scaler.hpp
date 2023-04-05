#pragma once

#include <cstdint>
#include <memory>

namespace hisui::video {

class YUVImage;

class Scaler {
 public:
  Scaler(const std::uint32_t t_width, const std::uint32_t t_height);
  virtual ~Scaler() = default;

  virtual const std::shared_ptr<YUVImage> scale(
      const std::shared_ptr<YUVImage> src) = 0;

 protected:
  std::shared_ptr<YUVImage> m_scaled;
  std::uint32_t m_width;
  std::uint32_t m_height;
};

}  // namespace hisui::video
