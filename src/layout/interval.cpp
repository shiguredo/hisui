#include <layout/interval.hpp>

namespace hisui::layout {

bool operator==(Interval const& left, Interval const& right) {
  return left.start_time == right.start_time && left.end_time == right.end_time;
}

std::ostream& operator<<(std::ostream& os, const Interval& i) {
  os << "start: " << i.start_time << " end: " << i.end_time;
  return os;
}

}  // namespace hisui::layout
