#pragma once

#include <codec/api/svc/codec_def.h>

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
};

}  // namespace hisui::video
