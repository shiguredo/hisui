#include "video/openh264.hpp"

#include <codec/api/wels/codec_def.h>

#include <algorithm>
#include <array>
#include <cstddef>
#include <cstdint>

#include "config.hpp"
#include "video/yuv.hpp"

namespace hisui::video {

void update_yuv_image_by_openh264_buffer_info(
    YUVImage* yuv_image,
    const ::SBufferInfo& buffer_info) {
  // buffer_info が準備できてない場合は更新しない
  if (buffer_info.iBufferStatus != 1) {
    return;
  }
  const std::uint32_t width0 =
      static_cast<std::uint32_t>(buffer_info.UsrData.sSystemBuffer.iWidth);
  const std::uint32_t height0 =
      static_cast<std::uint32_t>(buffer_info.UsrData.sSystemBuffer.iHeight);
  const std::uint32_t stride0 =
      static_cast<std::uint32_t>(buffer_info.UsrData.sSystemBuffer.iStride[0]);

  yuv_image->setWidthAndHeight(width0, height0);

  for (std::size_t i = 0; i < height0; ++i) {
    std::copy_n(buffer_info.pDst[0] + i * stride0, width0,
                yuv_image->yuv[0] + i * width0);
  }
  const auto width1 = (width0 + 1) >> 1;
  const auto height1 = (height0 + 1) >> 1;
  const std::uint32_t stride1 =
      static_cast<std::uint32_t>(buffer_info.UsrData.sSystemBuffer.iStride[1]);
  for (std::size_t i = 0; i < height1; ++i) {
    std::copy_n(buffer_info.pDst[1] + i * stride1, width1,
                yuv_image->yuv[1] + i * width1);
    std::copy_n(buffer_info.pDst[2] + i * stride1, width1,
                yuv_image->yuv[2] + i * width1);
  }
}

OpenH264EncoderConfig::OpenH264EncoderConfig(const std::uint32_t t_width,
                                             const std::uint32_t t_height,
                                             const hisui::Config& config)
    : width(t_width),
      height(t_height),
      fps(config.out_video_frame_rate),
      bitrate(config.out_video_bit_rate),
      threads(config.openh264_threads),
      min_qp(config.openh264_min_qp),
      max_qp(config.openh264_max_qp) {}

}  // namespace hisui::video
