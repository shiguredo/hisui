#include "video/preserve_aspect_ratio_scaler.hpp"

#include <bits/exception.h>
#include <spdlog/fmt/bundled/format.h>
#include <spdlog/spdlog.h>

#include <algorithm>
#include <array>
#include <cstdint>
#include <stdexcept>

#include <boost/cstdint.hpp>
#include <boost/exception/exception.hpp>
#include <boost/rational.hpp>

#include "video/yuv.hpp"

namespace hisui::video {

PreserveAspectRatioScaler::PreserveAspectRatioScaler(
    const std::uint32_t t_width,
    const std::uint32_t t_height,
    const libyuv::FilterMode t_filter_mode)
    : Scaler(t_width, t_height), m_filter_mode(t_filter_mode) {
  m_intermediate = std::make_shared<YUVImage>(m_width, m_height);
}

const std::shared_ptr<YUVImage> PreserveAspectRatioScaler::scale(
    const std::shared_ptr<YUVImage> src) {
  const auto src_width = src->getWidth(0);
  const auto src_height = src->getHeight(0);

  if (src_width == m_width && src_height == m_height) {
    return src;
  }

  boost::rational<std::uint32_t> width_ratio(m_width, src_width);
  boost::rational<std::uint32_t> height_ratio(m_height, src_height);

  if (width_ratio == height_ratio) {
    return simpleScale(src);
  }

  if (width_ratio > height_ratio) {
    return marginInWidthScale(src, src_width, height_ratio);
  }
  return marginInHeightScale(src, src_height, width_ratio);
}  // namespace hisui::video

const std::shared_ptr<YUVImage> PreserveAspectRatioScaler::marginInHeightScale(
    const std::shared_ptr<YUVImage> src,
    const std::uint32_t src_height,
    const boost::rational<std::uint32_t>& width_ratio) {
  const std::uint32_t intermediate_height =
      ((boost::rational_cast<std::uint32_t>(width_ratio * src_height) + 3) >> 2)
      << 2;
  if (intermediate_height >= m_height) {
    return simpleScale(src);
  }

  spdlog::trace("marginInHeightScale: {}x{}", m_width, intermediate_height);

  m_intermediate->setWidthAndHeight(m_width, intermediate_height);

  const int ret = libyuv::I420Scale(
      src->yuv[0], static_cast<int>(src->getWidth(0)), src->yuv[1],
      static_cast<int>(src->getWidth(1)), src->yuv[2],
      static_cast<int>(src->getWidth(2)), static_cast<int>(src->getWidth(0)),
      static_cast<int>(src->getHeight(0)), m_intermediate->yuv[0],
      static_cast<int>(m_intermediate->getWidth(0)), m_intermediate->yuv[1],
      static_cast<int>(m_intermediate->getWidth(1)), m_intermediate->yuv[2],
      static_cast<int>(m_intermediate->getWidth(2)),
      static_cast<int>(m_intermediate->getWidth(0)),
      static_cast<int>(m_intermediate->getHeight(0)), m_filter_mode);

  if (ret != 0) {
    throw std::runtime_error(
        fmt::format("I420Scale() failed: error_code={}", ret));
  }

  m_scaled->setBlack();

  std::copy_n(m_intermediate->yuv[0],
              m_intermediate->getWidth(0) * m_intermediate->getHeight(0),
              m_scaled->yuv[0] + ((m_height - intermediate_height) >> 1) *
                                     m_intermediate->getWidth(0));

  std::copy_n(m_intermediate->yuv[1],
              m_intermediate->getWidth(1) * m_intermediate->getHeight(1),
              m_scaled->yuv[1] + ((m_height - intermediate_height) >> 2) *
                                     m_intermediate->getWidth(1));

  std::copy_n(m_intermediate->yuv[2],
              m_intermediate->getWidth(2) * m_intermediate->getHeight(2),
              m_scaled->yuv[2] + ((m_height - intermediate_height) >> 2) *
                                     m_intermediate->getWidth(2));
  return m_scaled;
}

const std::shared_ptr<YUVImage> PreserveAspectRatioScaler::marginInWidthScale(
    const std::shared_ptr<YUVImage> src,
    const std::uint32_t src_width,
    const boost::rational<std::uint32_t>& height_ratio) {
  const std::uint32_t intermediate_width =
      ((boost::rational_cast<std::uint32_t>(height_ratio * src_width) + 3) >> 2)
      << 2;
  if (intermediate_width >= m_width) {
    return simpleScale(src);
  }

  spdlog::trace("marginInWidthScale: {}x{}", intermediate_width, m_height);

  m_intermediate->setWidthAndHeight(intermediate_width, m_height);

  const int ret = libyuv::I420Scale(
      src->yuv[0], static_cast<int>(src->getWidth(0)), src->yuv[1],
      static_cast<int>(src->getWidth(1)), src->yuv[2],
      static_cast<int>(src->getWidth(2)), static_cast<int>(src->getWidth(0)),
      static_cast<int>(src->getHeight(0)), m_intermediate->yuv[0],
      static_cast<int>(m_intermediate->getWidth(0)), m_intermediate->yuv[1],
      static_cast<int>(m_intermediate->getWidth(1)), m_intermediate->yuv[2],
      static_cast<int>(m_intermediate->getWidth(2)),
      static_cast<int>(m_intermediate->getWidth(0)),
      static_cast<int>(m_intermediate->getHeight(0)), m_filter_mode);

  if (ret != 0) {
    throw std::runtime_error(
        fmt::format("I420Scale() failed: error_code={}", ret));
  }

  m_scaled->setBlack();

  for (std::uint32_t h = 0, m = m_intermediate->getHeight(0); h < m; ++h) {
    std::copy_n(
        m_intermediate->yuv[0] + h * intermediate_width, intermediate_width,
        m_scaled->yuv[0] + m_width * h + ((m_width - intermediate_width) >> 1));
  }

  const auto m_width2 = m_scaled->getWidth(1);
  const auto intermediate_width2 = intermediate_width >> 1;
  for (std::uint32_t h = 0, m = m_intermediate->getHeight(1); h < m; ++h) {
    std::copy_n(m_intermediate->yuv[1] + h * intermediate_width2,
                intermediate_width2,
                m_scaled->yuv[1] + m_width2 * h +
                    ((m_width2 - intermediate_width2) >> 1));
  }

  for (std::uint32_t h = 0, m = m_intermediate->getHeight(2); h < m; ++h) {
    std::copy_n(m_intermediate->yuv[2] + h * intermediate_width2,
                intermediate_width2,
                m_scaled->yuv[2] + m_width2 * h +
                    ((m_width2 - intermediate_width2) >> 1));
  }

  return m_scaled;
}

const std::shared_ptr<YUVImage> PreserveAspectRatioScaler::simpleScale(
    const std::shared_ptr<YUVImage> src) {
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
