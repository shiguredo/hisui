#pragma once

#include <libyuv/scale.h>

#include <cstdint>
#include <memory>

#include "video/scaler.hpp"

namespace hisui::video {

class YUVImage;

class SimpleScaler : public Scaler {
 public:
  SimpleScaler(const std::uint32_t,
               const std::uint32_t,
               const libyuv::FilterMode);
  const std::shared_ptr<YUVImage> scale(const std::shared_ptr<YUVImage> src);

  const libyuv::FilterMode m_filter_mode;
};

}  // namespace hisui::video
