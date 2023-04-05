#pragma once

#include <cstdint>
#include <filesystem>
#include <memory>
#include <string>
#include <utility>
#include <vector>

#include "audio/source.hpp"
#include "layout/interval.hpp"
#include "util/interval.hpp"
#include "video/source.hpp"

namespace hisui::layout {

struct TrimIntervals {
  std::vector<Interval> trim_intervals;
};

bool operator==(TrimIntervals const& left, TrimIntervals const& right);

std::ostream& operator<<(std::ostream& os, const TrimIntervals&);

struct SourceParameters {
  const std::filesystem::path& file_path;
  const std::size_t index;
  const std::string& connection_id;
  const double start_time;
  const double end_time;
  const bool testing = false;
};

class Source {
 public:
  explicit Source(const SourceParameters&);
  virtual ~Source() = default;
  void substructTrimIntervals(const TrimIntervals&);
  bool hasConnectionID(const std::string&);
  bool hasIndex(const std::size_t);
  std::uint64_t getMaxEncodingTime() const;
  std::uint64_t getMinEncodingTime() const;
  void dump() const;
  void setEncodingInterval(const std::uint64_t);
  bool isIn(const std::uint64_t) const;
  std::size_t getIndex() const;
  Interval getSourceInterval() const;

 protected:
  std::filesystem::path m_file_path;
  std::size_t m_index;
  std::string m_connection_id;
  Interval m_source_interval;
  hisui::util::Interval m_encoding_interval{0, 0};
};

struct SubstructTrimIntervalsParameters {
  const Interval& interval;
  const std::vector<Interval>& trim_intervals;
};

Interval substruct_trim_intervals(const SubstructTrimIntervalsParameters&);

}  // namespace hisui::layout
