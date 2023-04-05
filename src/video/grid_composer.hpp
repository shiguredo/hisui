#pragma once

#include <libyuv/scale.h>

#include <array>
#include <cstddef>
#include <cstdint>
#include <memory>
#include <vector>

#include "config.hpp"
#include "video/composer.hpp"

namespace hisui::video {

class Scaler;
class YUVImage;

class GridComposer : public Composer {
 public:
  GridComposer(const std::uint32_t,
               const std::uint32_t,
               const std::size_t,
               const std::size_t,
               const hisui::config::VideoScaler&,
               const libyuv::FilterMode);

  ~GridComposer();

  void compose(std::vector<unsigned char>*,
               const std::vector<std::shared_ptr<YUVImage>>&);

 private:
  std::uint32_t m_single_width;
  std::uint32_t m_single_height;
  std::size_t m_size;
  std::size_t m_column;
  std::size_t m_row;
  std::array<unsigned char*, 3> m_planes;
  std::array<std::size_t, 3> m_plane_sizes;
  std::array<std::uint32_t, 3> m_single_plane_widths;
  std::array<std::uint32_t, 3> m_single_plane_heights;
  std::array<unsigned char, 3> m_plane_default_values;
  std::vector<std::shared_ptr<YUVImage>> m_scaled_images;
  std::vector<const unsigned char*> m_srcs;

  // Scaler::scale() は内部buffer を返すことがあるので, Source 分用意する
  std::vector<std::unique_ptr<Scaler>> m_scalers;
};

}  // namespace hisui::video
