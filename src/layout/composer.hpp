#pragma once

#include <array>
#include <cstddef>
#include <cstdint>
#include <memory>
#include <vector>

#include "layout/cell_util.hpp"

namespace hisui::layout {

class Region;

struct ComposerParameters {
  const std::vector<std::shared_ptr<Region>>& regions;
  const Resolution& resolution;
};

class Composer {
 public:
  explicit Composer(const ComposerParameters&);
  ~Composer();
  void compose(std::vector<unsigned char>*, const std::uint64_t);

 private:
  std::vector<std::shared_ptr<Region>> m_regions;
  Resolution m_resolution;

  std::array<unsigned char*, 3> m_planes;
  std::array<std::size_t, 3> m_plane_sizes;
  std::array<unsigned char, 3> m_plane_default_values;
};
}  // namespace hisui::layout
