#pragma once

#include <filesystem>
#include <memory>
#include <string>

#include <boost/json/value.hpp>

#include "archive_item.hpp"
#include "layout/interval.hpp"
#include "layout/source.hpp"

namespace hisui::layout {

struct ArchiveParameters {
  const std::filesystem::path& path;
  const std::filesystem::path& file_path;
  const std::string& connection_id;
  const double start_time;
  const double stop_time;
};

class Archive {
 public:
  explicit Archive(const ArchiveParameters&);

  void dump() const;
  const SourceParameters getSourceParameters(const std::size_t) const;
  void substructTrimIntervals(const TrimIntervals&);
  Interval getInterval() const;
  hisui::ArchiveItem getArchiveItem() const;

 private:
  std::filesystem::path m_path;
  std::filesystem::path m_file_path;
  std::string m_connection_id;
  double m_start_time;
  double m_stop_time;
};

std::shared_ptr<Archive> parse_archive(const std::string&);

}  // namespace hisui::layout
