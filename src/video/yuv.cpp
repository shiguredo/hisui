#include "video/yuv.hpp"

#include <algorithm>
#include <memory>

namespace hisui::video {

YUVImage::YUVImage(const std::uint32_t t_width, const std::uint32_t t_height)
    : m_width(t_width), m_height(t_height) {
  yuv[0] = new std::uint8_t[getWidth(0) * getHeight(0)];
  yuv[1] = new std::uint8_t[getWidth(1) * getHeight(1)];
  yuv[2] = new std::uint8_t[getWidth(2) * getHeight(2)];
}

YUVImage::~YUVImage() {
  delete[] yuv[0];
  delete[] yuv[1];
  delete[] yuv[2];
}

bool YUVImage::checkWidthAndHeight(const std::uint32_t t_width,
                                   const std::uint32_t t_height) const {
  return m_width == t_width && m_height == t_height;
}

void YUVImage::setWidthAndHeight(const std::uint32_t t_width,
                                 const std::uint32_t t_height) {
  if (checkWidthAndHeight(t_width, t_height)) {
    return;
  }
  if (t_width * t_height == m_width * m_height) {
    m_width = t_width;
    m_height = t_height;
    return;
  }
  m_width = t_width;
  m_height = t_height;
  delete[] yuv[0];
  delete[] yuv[1];
  delete[] yuv[2];
  yuv[0] = new std::uint8_t[getWidth(0) * getHeight(0)];
  yuv[1] = new std::uint8_t[getWidth(1) * getHeight(1)];
  yuv[2] = new std::uint8_t[getWidth(2) * getHeight(2)];
}

std::uint32_t YUVImage::getWidth(const int plane) const {
  if (plane == 0) {
    return m_width;
  }
  return (m_width + 1) >> 1;
}

std::uint32_t YUVImage::getHeight(const int plane) const {
  if (plane == 0) {
    return m_height;
  }
  return (m_height + 1) >> 1;
}

void YUVImage::setBlack() {
  for (std::size_t i = 0; i < 3; ++i) {
    std::fill(
        yuv[i],
        yuv[i] + getWidth(static_cast<int>(i)) * getHeight(static_cast<int>(i)),
        i == 0 ? 0 : 128);
  }
}

std::shared_ptr<YUVImage> create_black_yuv_image(const std::uint32_t width,
                                                 const std::uint32_t height) {
  std::shared_ptr<YUVImage> image = std::make_shared<YUVImage>(width, height);
  image->setBlack();
  return image;
}

void merge_yuv_planes_from_top_left(
    unsigned char* merged,
    const std::size_t merged_size,
    const std::size_t column,
    const std::vector<const unsigned char*>& srcs,
    const std::size_t number_of_srcs,
    const std::uint32_t src_width,
    const std::uint32_t src_height,
    const unsigned char default_value) {
  std::fill_n(merged, merged_size, default_value);

  for (std::size_t i = 0; i < number_of_srcs; ++i) {
    const auto c = i % column;
    const auto r = i / column;
    for (std::uint32_t y = 0; y < src_height; ++y) {
      std::copy_n(srcs[i] + y * src_width, src_width,
                  merged + r * src_width * src_height * column + c * src_width +
                      y * column * src_width);
    }
  }
}

void overlay_yuv_planes(unsigned char* overlayed,
                        const unsigned char* src,
                        const std::uint32_t base_width,
                        const std::uint32_t src_x,
                        const std::uint32_t src_y,
                        const std::uint32_t src_width,
                        const std::uint32_t src_height) {
  for (std::uint32_t y = 0; y < src_height; ++y) {
    std::copy_n(src + y * src_width, src_width,
                overlayed + (src_y + y) * base_width + src_x);
  }
}

}  // namespace hisui::video
