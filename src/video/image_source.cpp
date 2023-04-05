#include "video/image_source.hpp"

#include <bits/exception.h>
#include <fmt/core.h>
#include <libyuv/convert.h>
#include <spdlog/fmt/fmt.h>
#include <spdlog/spdlog.h>
#define STB_IMAGE_IMPLEMENTATION
#include <stb_image.h>

#include <array>
#include <stdexcept>

#include "video/yuv.hpp"

namespace hisui::video {

ImageSource::ImageSource(const std::string& file_path) {
  int width, height, comp;
  unsigned char* data = stbi_load(file_path.c_str(), &width, &height, &comp, 4);
  m_width = static_cast<std::uint32_t>(width);
  m_height = static_cast<std::uint32_t>(height);

  spdlog::trace("ImageSource file_path={}, width={}, height={}", file_path,
                m_width, m_height);

  m_yuv_image = std::make_shared<YUVImage>(m_width, m_height);

  const int width2 = (m_width + 1) >> 1;
  const auto ret = libyuv::ABGRToI420(
      reinterpret_cast<const std::uint8_t*>(data), width * 4,
      m_yuv_image->yuv[0], static_cast<int>(m_width), m_yuv_image->yuv[1],
      width2, m_yuv_image->yuv[2], width2, width, height);
  stbi_image_free(data);

  if (ret != 0) {
    throw std::runtime_error(
        fmt::format("libyuv::ARGBToI420() failed: file_path={}", file_path));
  }
}

const std::shared_ptr<YUVImage> ImageSource::getYUV(const std::uint64_t) {
  return m_yuv_image;
}

std::uint32_t ImageSource::getWidth() const {
  return m_width;
}
std::uint32_t ImageSource::getHeight() const {
  return m_height;
}

}  // namespace hisui::video
