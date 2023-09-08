#pragma once

#include <codec/api/wels/codec_app_def.h>
#include <codec/api/wels/codec_def.h>

#include <cstdint>

#include <boost/rational.hpp>

namespace hisui {

class Config;

}

namespace hisui::video {

class YUVImage;

void update_yuv_image_by_openh264_buffer_info(YUVImage*, const ::SBufferInfo&);

class OpenH264EncoderConfig {
 public:
  OpenH264EncoderConfig(const std::uint32_t,
                        const std::uint32_t,
                        const hisui::Config&);
  const std::uint32_t width;
  const std::uint32_t height;
  const boost::rational<std::uint64_t> fps;
  const std::uint32_t bitrate;
  const std::uint16_t threads;
  const std::int32_t min_qp;
  const std::int32_t max_qp;
  const ::EProfileIdc profile = ::PRO_BASELINE;
  const ::ELevelIdc level = ::LEVEL_3_1;
};

}  // namespace hisui::video
