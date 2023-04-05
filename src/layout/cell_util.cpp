#include "layout/cell_util.hpp"

#include <cstdint>
#include <ostream>
#include <stdexcept>
#include <vector>

namespace hisui::layout {

bool operator==(Position const& left, Position const& right) {
  return left.x == right.x && left.y == right.y;
}

std::ostream& operator<<(std::ostream& os, const Position& p) {
  os << "x: " << p.x << " y: " << p.y;
  return os;
}

bool operator==(Resolution const& left, Resolution const& right) {
  return left.width == right.width && left.height == right.height;
}

std::ostream& operator<<(std::ostream& os, const Resolution& r) {
  os << "width: " << r.width << " height: " << r.height;
  return os;
}

bool operator==(LengthAndPositions const& left,
                LengthAndPositions const& right) {
  return left.length == right.length && left.positions == right.positions;
}

std::ostream& operator<<(std::ostream& os, const LengthAndPositions& lp) {
  os << "length: " << lp.length;
  os << " positions: [";
  for (const auto i : lp.positions) {
    os << " " << i << " ";
  }
  os << "]";
  return os;
}

bool operator==(ResolutionAndPositions const& left,
                ResolutionAndPositions const& right) {
  return left.resolution == right.resolution &&
         left.positions == right.positions;
}

std::ostream& operator<<(std::ostream& os, const ResolutionAndPositions& rp) {
  os << "resolution: " << rp.resolution;
  os << " positions: [";
  for (const auto i : rp.positions) {
    os << " {" << i << "} ";
  }
  os << "]";
  return os;
}

LengthAndPositions calc_cell_length_and_positions(
    const CalcCellLengthAndPositions& params) {
  if (params.number_of_cells == 0) {
    throw std::invalid_argument("number_of_cells should be grater than 0");
  }
  if (params.min_frame_length % 2) {
    throw std::invalid_argument("min_frame_width should be even number");
  }
  // 動画の描画に利用できる長さ
  auto allLength = params.region_length - (params.is_frame_on_ends
                                               ? (params.number_of_cells + 1)
                                               : (params.number_of_cells - 1)) *
                                              params.min_frame_length;
  // 1 動画の長さ (4の倍数に補正)
  auto length = ((allLength / params.number_of_cells) >> 2) << 2;

  // 各動画の位置を算出
  std::vector<std::uint32_t> positions;
  for (std::uint64_t i = 0; i < params.number_of_cells; ++i) {
    positions.emplace_back(params.min_frame_length *
                               (params.is_frame_on_ends ? (i + 1) : i) +
                           length * i);
  }
  return {.length = length, .positions = positions};
}

ResolutionAndPositions calc_cell_resolution_and_positions(
    const CalcCellResolutionAndPositions& params) {
  auto side = calc_cell_length_and_positions({
      .number_of_cells = params.grid_dimension.columns,
      .region_length = params.region_resolution.width,
      .min_frame_length = params.min_frame_width,
      .is_frame_on_ends = params.is_width_frame_on_ends,
  });
  auto vert = calc_cell_length_and_positions({
      .number_of_cells = params.grid_dimension.rows,
      .region_length = params.region_resolution.height,
      .min_frame_length = params.min_frame_height,
      .is_frame_on_ends = params.is_height_frame_on_ends,
  });

  std::vector<Position> positions;
  for (auto y = 0u; y < params.grid_dimension.rows; ++y) {
    for (auto x = 0u; x < params.grid_dimension.columns; ++x) {
      positions.push_back({.x = side.positions[x], .y = vert.positions[y]});
    }
  }
  return {.resolution = {.width = side.length, .height = vert.length},
          .positions = positions};
}

}  // namespace hisui::layout
