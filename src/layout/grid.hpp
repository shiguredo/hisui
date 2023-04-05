#pragma once

#include <cstdint>
#include <ostream>
#include <vector>

namespace hisui::layout {

struct GridDimension {
  std::uint32_t columns;
  std::uint32_t rows;
};

bool operator==(GridDimension const& left, GridDimension const& right);

std::ostream& operator<<(std::ostream& os, const GridDimension& gd);

struct CalcGridDimensionParameters {
  std::uint32_t max_columns = 0;  // 0: unconstrained
  std::uint32_t max_rows = 0;     // 0: unconstrained
  std::uint32_t number_of_sources = 0;
};

GridDimension calc_grid_dimension(const CalcGridDimensionParameters&);

struct AddNumberOfExcludedCellsParameters {
  std::uint32_t number_of_sources = 0;
  const std::vector<std::uint64_t>& cells_excluded = {};  // sorted
};

std::uint32_t add_number_of_excluded_cells(
    const AddNumberOfExcludedCellsParameters&);

}  // namespace hisui::layout
