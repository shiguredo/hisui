#pragma once

#include <cstdint>
#include <stdexcept>
#include <vector>

namespace hisui::video {

class Encoder {
 public:
  virtual ~Encoder() = default;
  virtual void outputImage(const std::vector<unsigned char>&) = 0;
  virtual void flush() = 0;
  virtual void setResolutionAndBitrate(const std::uint32_t,
                                       const std::uint32_t,
                                       const std::uint32_t) {}

  virtual std::uint32_t getFourcc() const = 0;
  virtual const std::vector<std::uint8_t>& getExtraData() const {
    throw std::logic_error("Encoder::getExtraData() should not be called");
  }
};

}  // namespace hisui::video
