#include "video/simple_scaler.hpp"

#include <fmt/core.h>
#include <libyuv/scale.h>

#include <array>
#include <stdexcept>

#include "video/yuv.hpp"

namespace hisui::video {

SimpleScaler::SimpleScaler(const std::uint32_t t_width,
                           const std::uint32_t t_height,
                           const libyuv::FilterMode t_filter_mode)
    : Scaler(t_width, t_height), m_filter_mode(t_filter_mode) {}

const std::shared_ptr<YUVImage> SimpleScaler::scale(
    const std::shared_ptr<YUVImage> src) {
  if (src->getWidth(0) == m_width && src->getHeight(0) == m_height) {
    return src;
  }
  const int ret = libyuv::I420Scale(
      src->yuv[0], static_cast<int>(src->getWidth(0)), src->yuv[1],
      static_cast<int>(src->getWidth(1)), src->yuv[2],
      static_cast<int>(src->getWidth(2)), static_cast<int>(src->getWidth(0)),
      static_cast<int>(src->getHeight(0)), m_scaled->yuv[0],
      static_cast<int>(m_scaled->getWidth(0)), m_scaled->yuv[1],
      static_cast<int>(m_scaled->getWidth(1)), m_scaled->yuv[2],
      static_cast<int>(m_scaled->getWidth(2)),
      static_cast<int>(m_scaled->getWidth(0)),
      static_cast<int>(m_scaled->getHeight(0)), m_filter_mode);

  if (ret != 0) {
    throw std::runtime_error(
        fmt::format("I420Scale() failed: error_code={}", ret));
  }
  return m_scaled;
}

}  // namespace hisui::video
