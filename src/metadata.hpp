#pragma once

#include <filesystem>
#include <limits>
#include <string>
#include <vector>

#include <boost/json/array.hpp>
#include <boost/json/object.hpp>
#include <boost/json/string.hpp>
#include <boost/json/value.hpp>

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

  Archive& operator=(const Archive& other);

 private:
  std::filesystem::path m_path;
  std::string m_connection_id;
  double m_start_time_offset;
  double m_stop_time_offset;
};

class Metadata {
 public:
  Metadata(const std::string&, const boost::json::value&);
  explicit Metadata(const std::vector<Archive>&);
  std::vector<Archive> getArchives() const;
  double getMinStartTimeOffset() const;
  double getMaxStopTimeOffset() const;
  double getCreatedAt() const;
  void adjustTimeOffsets(double);
  std::filesystem::path getPath() const;
  boost::json::string getRecordingID() const;

 private:
  void setTimeOffsets();
  boost::json::array prepare(const boost::json::value& jv);

  std::filesystem::path m_path;
  std::vector<Archive> m_archives;
  double m_min_start_time_offset = std::numeric_limits<double>::max();
  double m_max_stop_time_offset = std::numeric_limits<double>::min();
  double m_created_at;
  boost::json::string m_recording_id;
};

Metadata parse_metadata(const std::string&);

class MetadataSet {
 public:
  explicit MetadataSet(const Metadata&);
  void setPrefered(const Metadata&);
  Metadata getNormal() const;
  Metadata getPreferred() const;
  bool hasPreferred() const;
  std::vector<Archive> getArchives() const;
  std::vector<Archive> getNormalArchives() const;
  double getMaxStopTimeOffset() const;

 private:
  Metadata m_normal;
  Metadata m_preferred;
  bool m_has_preferred = false;
};
Metadata parse_metadata(const std::string&, const boost::json::value&);
boost::json::string get_string_from_json_object(boost::json::object o,
                                                const std::string& key);
double get_double_from_json_object(boost::json::object o,
                                   const std::string& key);

}  // namespace hisui
