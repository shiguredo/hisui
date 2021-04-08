#pragma once

#include <filesystem>
#include <limits>
#include <string>
#include <vector>

#include <boost/json.hpp>

namespace hisui {

class Archive {
 public:
  Archive(const std::filesystem::path&,
          const std::string&,
          const double,
          const double);

  std::filesystem::path getPath() const;
  std::string getConnectionID() const;
  double getStartTimeOffset() const;
  double getStopTimeOffset() const;
  void adjustTimeOffsets(double);

 private:
  const std::filesystem::path m_path;
  const std::string m_connection_id;
  double m_start_time_offset;
  double m_stop_time_offset;
};

class Metadata {
 public:
  Metadata(const std::string&, const boost::json::array&);
  explicit Metadata(const std::vector<Archive>&);  // for testing
  std::vector<Archive> getArchives() const;
  double getMinStartTimeOffset() const;
  double getMaxStopTimeOffset() const;
  void adjustTimeOffsets(double);
  std::filesystem::path getPath() const;

 private:
  void setTimeOffsets();

  std::filesystem::path m_path;
  std::vector<Archive> m_archives;
  double m_min_start_time_offset = std::numeric_limits<double>::max();
  double m_max_stop_time_offset = std::numeric_limits<double>::min();
};

Metadata parse_metadata(const std::string&, const boost::json::value&);

}  // namespace hisui
