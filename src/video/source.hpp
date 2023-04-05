#pragma once

#include <cstdint>
#include <memory>

namespace hisui::video {

class YUVImage;

class Source {
 public:
  virtual ~Source() = default;
  virtual const std::shared_ptr<YUVImage> getYUV(const std::uint64_t) = 0;
  virtual std::uint32_t getWidth() const = 0;
  virtual std::uint32_t getHeight() const = 0;
};

}  // namespace hisui::video
