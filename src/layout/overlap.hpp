#pragma once

#include <cstdint>
#include <iosfwd>
#include <list>
#include <vector>

#include "layout/interval.hpp"
#include "layout/reuse.hpp"
#include "layout/source.hpp"

namespace hisui::layout {

struct OverlapIntervalsParameters {
  const std::vector<Interval>& intervals;
  Reuse reuse;
};

struct OverlapIntervalsResult {
  std::uint32_t max_number_of_overlap;
  double min_start_time;
  double max_end_time;
  std::vector<Interval> trim_intervals;
};

bool operator==(OverlapIntervalsResult const& left,
                OverlapIntervalsResult const& right);

std::ostream& operator<<(std::ostream& os, const OverlapIntervalsResult&);

OverlapIntervalsResult overlap_intervals(const OverlapIntervalsParameters&);

struct OverlapTrimIntervalsParameters {
  const std::list<std::vector<Interval>>& list_of_trim_intervals;
};

TrimIntervals overlap_trim_intervals(const OverlapTrimIntervalsParameters&);

}  // namespace hisui::layout
