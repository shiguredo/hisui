#include "layout/source.hpp"

#include <fmt/core.h>
#include <spdlog/spdlog.h>

#include <algorithm>
#include <limits>

#include "audio/webm_source.hpp"
#include "layout/interval.hpp"
#include "layout/overlap.hpp"

namespace hisui::layout {

bool operator==(TrimIntervals const& left, TrimIntervals const& right) {
  return left.trim_intervals == right.trim_intervals;
}

std::ostream& operator<<(std::ostream& os, const TrimIntervals& r) {
  os << "[";
  for (const auto& i : r.trim_intervals) {
    os << " {" << i.start_time << ", " << i.end_time << "} ";
  }
  os << "]";
  return os;
}

Source::Source(const SourceParameters& params)
    : m_file_path(params.file_path),
      m_index(params.index),
      m_connection_id(params.connection_id),
      m_source_interval{params.start_time, params.end_time} {}

void Source::substructTrimIntervals(const TrimIntervals& params) {
  m_source_interval = substruct_trim_intervals(
      {.interval = m_source_interval, .trim_intervals = params.trim_intervals});
}

Interval substruct_trim_intervals(
    const SubstructTrimIntervalsParameters& params) {
  auto interval = params.interval;
  auto trims = params.trim_intervals;
  if (std::empty(trims)) {
    return interval;
  }
  for (std::int64_t i = static_cast<std::int64_t>(std::size(trims)) - 1; i >= 0;
       --i) {
    auto s = static_cast<std::size_t>(i);

    if (trims[s].start_time >= interval.end_time) {
      continue;
    }
    auto t = trims[s].end_time - trims[s].start_time;
    interval.start_time -= t;
    interval.end_time -= t;
  }
  return interval;
}

bool Source::hasConnectionID(const std::string& connection_id) {
  return m_connection_id == connection_id;
}

bool Source::hasIndex(const std::size_t index) {
  return m_index == index;
}

std::uint64_t Source::getMaxEncodingTime() const {
  return m_encoding_interval.getUpper();
}

std::uint64_t Source::getMinEncodingTime() const {
  return m_encoding_interval.getLower();
}

void Source::dump() const {
  spdlog::debug("    file_path: {}", m_file_path.string());
  spdlog::debug("    connection_id: {}", m_connection_id);
  spdlog::debug("    start_time: {}", m_source_interval.start_time);
  spdlog::debug("    end_time: {}", m_source_interval.end_time);
}

void Source::setEncodingInterval(const std::uint64_t timescale) {
  m_encoding_interval.set(
      static_cast<std::uint64_t>(std::floor(m_source_interval.start_time *
                                            static_cast<double>(timescale))),
      static_cast<std::uint64_t>(std::ceil(m_source_interval.end_time *
                                           static_cast<double>(timescale))));
}

bool Source::isIn(const std::uint64_t t) const {
  return m_encoding_interval.isIn(t);
}

std::size_t Source::getIndex() const {
  return m_index;
}

Interval Source::getSourceInterval() const {
  return m_source_interval;
}
}  // namespace hisui::layout
