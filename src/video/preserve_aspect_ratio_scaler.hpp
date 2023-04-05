#pragma once

#include <cstdint>
#include <memory>

#include "libyuv/scale.h"

#include "video/scaler.hpp"

namespace boost {

template <typename IntType>
class rational;

}

namespace hisui::video {

class YUVImage;

class PreserveAspectRatioScaler : public Scaler {
 public:
  PreserveAspectRatioScaler(const std::uint32_t,
                            const std::uint32_t,
                            const libyuv::FilterMode);
  const std::shared_ptr<YUVImage> scale(const std::shared_ptr<YUVImage>);

 private:
  const libyuv::FilterMode m_filter_mode;
  std::shared_ptr<YUVImage> m_intermediate;

  const std::shared_ptr<YUVImage> simpleScale(const std::shared_ptr<YUVImage>);
  const std::shared_ptr<YUVImage> marginInHeightScale(
      const std::shared_ptr<YUVImage>,
      const std::uint32_t,
      const boost::rational<std::uint32_t>&);
  const std::shared_ptr<YUVImage> marginInWidthScale(
      const std::shared_ptr<YUVImage>,
      const std::uint32_t,
      const boost::rational<std::uint32_t>&);
};

}  // namespace hisui::video
