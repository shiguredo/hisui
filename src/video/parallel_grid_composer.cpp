#include "video/parallel_grid_composer.hpp"

#include <cxxabi.h>
#include <libyuv/scale.h>

#include <algorithm>
#include <future>
#include <system_error>

#include "video/preserve_aspect_ratio_scaler.hpp"
#include "video/scaler.hpp"
#include "video/simple_scaler.hpp"
#include "video/yuv.hpp"

namespace hisui::video {

ParallelGridComposer::ParallelGridComposer(
    const std::uint32_t t_single_width,
    const std::uint32_t t_single_height,
    const std::size_t t_size,
    const std::size_t t_colomn,
    const hisui::config::VideoScaler& scaler_type,
    const libyuv::FilterMode filter_mode)
    : m_single_width(t_single_width),
      m_single_height(t_single_height),
      m_size(t_size),
      m_column(t_colomn) {
  m_column = std::min(m_column, m_size);
  m_row = m_column == 1 ? m_size : ((m_size + m_column - 1) / m_column);
  m_width = static_cast<std::uint32_t>(m_single_width * m_column);
  m_height = static_cast<std::uint32_t>(m_single_height * m_row);
  for (std::size_t i = 0; i < m_size; ++i) {
    switch (scaler_type) {
      case hisui::config::VideoScaler::PreserveAspectRatio:
        m_scalers.push_back(std::make_unique<PreserveAspectRatioScaler>(
            m_single_width, m_single_height, filter_mode));
        break;
      case hisui::config::VideoScaler::Simple:
        m_scalers.push_back(std::make_unique<SimpleScaler>(
            m_single_width, m_single_height, filter_mode));
        break;
    }
  }
  m_scaled_images.resize(m_size);
  m_srcs[0].resize(m_size);
  m_srcs[1].resize(m_size);
  m_srcs[2].resize(m_size);
  m_plane_sizes[0] = m_width * m_height;
  m_plane_sizes[1] = (m_plane_sizes[0] + 3) >> 2;
  m_plane_sizes[2] = m_plane_sizes[1];
  m_planes[0] = new unsigned char[m_plane_sizes[0]];
  m_planes[1] = new unsigned char[m_plane_sizes[1]];
  m_planes[2] = new unsigned char[m_plane_sizes[2]];

  m_single_plane_widths[0] = m_single_width;
  m_single_plane_widths[1] = (m_single_width + 1) >> 1;
  m_single_plane_widths[2] = m_single_plane_widths[1];
  m_single_plane_heights[0] = m_single_height;
  m_single_plane_heights[1] = (m_single_height + 1) >> 1;
  m_single_plane_heights[2] = m_single_plane_heights[1];
  m_plane_default_values[0] = 0;
  m_plane_default_values[1] = 128;
  m_plane_default_values[2] = 128;
}

ParallelGridComposer::~ParallelGridComposer() {
  for (std::size_t p = 0; p < 3; ++p) {
    delete[] m_planes[p];
  }
}

void ParallelGridComposer::compose(std::vector<unsigned char>* composed,
                                   const std::vector<const YUVImage*>& images) {
  for (std::size_t i = 0; i < m_size; ++i) {
    m_scaled_images[i] = m_scalers[i]->scale(images[i]);
  }

  auto future0 = std::async(std::launch::async, [this, composed] {
    for (std::size_t i = 0; i < m_size; ++i) {
      m_srcs[0][i] = m_scaled_images[i]->yuv[0];
    }
    merge_yuv_planes_from_top_left(m_planes[0], m_plane_sizes[0], m_column,
                                   m_srcs[0], m_size, m_single_plane_widths[0],
                                   m_single_plane_heights[0],
                                   m_plane_default_values[0]);

    std::copy_n(m_planes[0], m_plane_sizes[0], composed->data());
  });

  auto future1 = std::async(std::launch::async, [this, composed] {
    for (std::size_t i = 0; i < m_size; ++i) {
      m_srcs[1][i] = m_scaled_images[i]->yuv[1];
    }
    merge_yuv_planes_from_top_left(m_planes[1], m_plane_sizes[1], m_column,
                                   m_srcs[1], m_size, m_single_plane_widths[1],
                                   m_single_plane_heights[1],
                                   m_plane_default_values[1]);

    std::copy_n(m_planes[1], m_plane_sizes[1],
                composed->data() + m_plane_sizes[0]);
  });

  auto future2 = std::async(std::launch::async, [this, composed] {
    for (std::size_t i = 0; i < m_size; ++i) {
      m_srcs[2][i] = m_scaled_images[i]->yuv[2];
    }
    merge_yuv_planes_from_top_left(m_planes[2], m_plane_sizes[2], m_column,
                                   m_srcs[2], m_size, m_single_plane_widths[2],
                                   m_single_plane_heights[2],
                                   m_plane_default_values[2]);

    std::copy_n(m_planes[2], m_plane_sizes[2],
                composed->data() + m_plane_sizes[0] + m_plane_sizes[1]);
  });

  future0.get();
  future1.get();
  future2.get();
}

}  // namespace hisui::video
