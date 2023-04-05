#pragma once

#include <cstdint>
#include <iosfwd>
#include <vector>

#include "layout/grid.hpp"

namespace hisui::layout {

struct Position {
  std::uint32_t x = 0;
  std::uint32_t y = 0;
};

bool operator==(Position const& left, Position const& right);

std::ostream& operator<<(std::ostream& os, const Position&);

struct Resolution {
  std::uint32_t width = 0;
  std::uint32_t height = 0;
};

bool operator==(Resolution const& left, Resolution const& right);

std::ostream& operator<<(std::ostream& os, const Resolution&);

struct CalcCellLengthAndPositions {
  const std::uint32_t number_of_cells;
  const std::uint32_t region_length;
  const std::uint32_t min_frame_length;
  const bool is_frame_on_ends = true;
};

struct LengthAndPositions {
  std::uint32_t length;
  std::vector<std::uint32_t> positions;
};

bool operator==(LengthAndPositions const& left,
                LengthAndPositions const& right);

std::ostream& operator<<(std::ostream& os, const LengthAndPositions&);

LengthAndPositions calc_cell_length_and_positions(
    const CalcCellLengthAndPositions&);

struct CalcCellResolutionAndPositions {
  const GridDimension grid_dimension;
  const Resolution region_resolution;
  const std::uint32_t min_frame_width = 2;
  const std::uint32_t min_frame_height = 2;
  const bool is_width_frame_on_ends = true;
  const bool is_height_frame_on_ends = true;
};

struct ResolutionAndPositions {
  Resolution resolution;
  std::vector<Position> positions;
};

bool operator==(ResolutionAndPositions const& left,
                ResolutionAndPositions const& right);

std::ostream& operator<<(std::ostream& os, const ResolutionAndPositions&);

ResolutionAndPositions calc_cell_resolution_and_positions(
    const CalcCellResolutionAndPositions&);

}  // namespace hisui::layout
