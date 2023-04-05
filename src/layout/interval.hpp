#pragma once

#include <ostream>

namespace hisui::layout {

struct Interval {
  double start_time;
  double end_time;
};

bool operator==(Interval const& left, Interval const& right);

std::ostream& operator<<(std::ostream& os, const Interval&);

}  // namespace hisui::layout
