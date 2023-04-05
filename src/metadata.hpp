#pragma once

#include <filesystem>
#include <limits>
#include <string>
#include <vector>

#include <boost/json/array.hpp>
#include <boost/json/object.hpp>
#include <boost/json/string.hpp>
#include <boost/json/value.hpp>

#include "archive_item.hpp"

namespace hisui {

class Metadata {
 public:
  Metadata(const std::string&, const boost::json::value&);
  explicit Metadata(const std::vector<ArchiveItem>&);

  std::vector<ArchiveItem> getArchiveItems() const;
  double getMinStartTimeOffset() const;
  double getMaxStopTimeOffset() const;
  double getCreatedAt() const;
  std::filesystem::path getPath() const;
  boost::json::string getRecordingID() const;

  void adjustTimeOffsets(double);
  void copyWithoutArchives(const Metadata&);
  void setArchives(const std::vector<ArchiveItem>&);
  std::vector<ArchiveItem> deleteArchivesByConnectionID(const std::string&);

 private:
  boost::json::array prepare(const boost::json::value& jv);
  void setTimeOffsets();

  std::filesystem::path m_path;
  std::vector<ArchiveItem> m_archives;
  double m_min_start_time_offset = std::numeric_limits<double>::max();
  double m_max_stop_time_offset = std::numeric_limits<double>::min();
  boost::json::string m_recording_id;
  double m_created_at;
};

Metadata parse_metadata(const std::string&);

class MetadataSet {
 public:
  explicit MetadataSet(const Metadata&);
  void setPrefered(const Metadata&);
  void split(const std::string&);
  Metadata getNormal() const;
  Metadata getPreferred() const;
  bool hasPreferred() const;
  std::vector<ArchiveItem> getNormalArchives() const;
  std::vector<ArchiveItem> getArchiveItems() const;
  double getMaxStopTimeOffset() const;

 private:
  Metadata m_normal;
  Metadata m_preferred;
  bool m_has_preferred = false;
};

}  // namespace hisui
