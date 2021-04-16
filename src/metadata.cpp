#include "metadata.hpp"

#include <bits/exception.h>
#include <spdlog/fmt/bundled/format.h>
#include <spdlog/spdlog.h>

#include <algorithm>
#include <compare>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <stdexcept>
#include <tuple>
#include <utility>

#include <boost/json/impl/array.hpp>
#include <boost/json/parse.hpp>
#include <boost/json/system_error.hpp>

namespace hisui {

Archive::Archive(const std::filesystem::path& t_path,
                 const std::string& m_connection_id,
                 const double t_start_time_offset,
                 const double t_stop_time_offset)
    : m_path(t_path),
      m_connection_id(m_connection_id),
      m_start_time_offset(t_start_time_offset),
      m_stop_time_offset(t_stop_time_offset) {}

std::filesystem::path Archive::getPath() const {
  return m_path;
}

std::string Archive::getConnectionID() const {
  return m_connection_id;
}

double Archive::getStartTimeOffset() const {
  return m_start_time_offset;
}

double Archive::getStopTimeOffset() const {
  return m_stop_time_offset;
}

void Archive::adjustTimeOffsets(double diff) {
  m_start_time_offset += diff;
  m_stop_time_offset += diff;
}

Archive& Archive::operator=(const Archive& other) {
  if (this != &other) {
    this->m_path = other.m_path;
    this->m_connection_id = other.m_connection_id;
    this->m_start_time_offset = other.m_start_time_offset;
    this->m_stop_time_offset = other.m_stop_time_offset;
  }
  return *this;
}

Metadata::Metadata(const std::string& file_path, const boost::json::value& jv)
    : m_path(file_path) {
  if (m_path.is_relative()) {
    m_path = std::filesystem::absolute(m_path);
  }
  const auto current_path = std::filesystem::current_path();
  std::filesystem::current_path(m_path.parent_path());
  std::vector<std::tuple<std::string, std::string, double, double>> archives;

  auto json_archives = prepare(jv);

  for (const auto& a : json_archives) {
    boost::json::object o;
    if (auto p = a.if_object()) {
      o = *p;
    } else {
      throw std::runtime_error("a.if_object() failed");
    }
    auto a_file_path = get_string_from_json_object(o, "file_path");
    auto a_connection_id = get_string_from_json_object(o, "connection_id");
    double a_start_time_offset =
        get_double_from_json_object(o, "start_time_offset");
    double a_stop_time_offset =
        get_double_from_json_object(o, "stop_time_offset");
    spdlog::debug("{} {} {} {}", a_file_path, a_connection_id,
                  a_start_time_offset, a_stop_time_offset);
    archives.emplace_back(a_file_path, a_connection_id, a_start_time_offset,
                          a_stop_time_offset);
  }
  std::sort(archives.begin(), archives.end(),
            [](const std::tuple<std::string, std::string, double, double>& a,
               const std::tuple<std::string, std::string, double, double>& b) {
              if (get<2>(a) != get<2>(b)) {
                // 開始時間が先のものを優先する
                return get<2>(a) < get<2>(b);
              }
              if (get<3>(a) != get<3>(b)) {
                // 終了時間が後のものを優先する
                return get<3>(a) > get<3>(b);
              }
              if (get<1>(a) != get<1>(b)) {
                return get<1>(a) < get<1>(b);
              }
              if (get<0>(a) != get<0>(b)) {
                return get<0>(a) < get<0>(b);
              }
              return false;
            });

  for (const auto& a : archives) {
    std::filesystem::path path(get<0>(a));
    if (path.is_relative()) {
      path = std::filesystem::absolute(path);
    }
    if (!std::filesystem::exists(path)) {
      spdlog::debug("file is not found(1). try relative path: {}",
                    path.string());
      path = std::filesystem::absolute(path.filename());
      if (!std::filesystem::exists(path)) {
        spdlog::debug("file is not found(2): {}", path.string());
        throw std::runtime_error(
            fmt::format("file is not found: {}", get<0>(a)));
      }
    }
    Archive archive(path, get<1>(a), get<2>(a), get<3>(a));
    m_archives.push_back(archive);
  }
  setTimeOffsets();
  std::filesystem::current_path(current_path);
}

Metadata::Metadata(const std::vector<Archive>& t_archives)
    : m_archives(t_archives) {
  setTimeOffsets();
}

void Metadata::setTimeOffsets() {
  for (const auto& archive : m_archives) {
    if (archive.getStartTimeOffset() < m_min_start_time_offset) {
      m_min_start_time_offset = archive.getStartTimeOffset();
    }
    if (archive.getStopTimeOffset() > m_max_stop_time_offset) {
      m_max_stop_time_offset = archive.getStopTimeOffset();
    }
  }
}

std::vector<Archive> Metadata::getArchives() const {
  return m_archives;
}

double Metadata::getMinStartTimeOffset() const {
  return m_min_start_time_offset;
}

double Metadata::getMaxStopTimeOffset() const {
  return m_max_stop_time_offset;
}

double Metadata::getCreatedAt() const {
  return m_created_at;
}

boost::json::array Metadata::prepare(const boost::json::value& jv) {
  boost::json::object j;
  if (auto p = jv.if_object()) {
    j = *p;
  } else {
    throw std::runtime_error("jv.if_object() failed");
  }

  m_recording_id = get_string_from_json_object(j, "recording_id");
  m_created_at = get_double_from_json_object(j, "created_at");

  if (j["archives"] == nullptr) {
    throw std::invalid_argument("not metadata json file: {}");
  }

  boost::json::array ja;
  if (auto p = j["archives"].if_array()) {
    ja = *p;
  } else {
    throw std::runtime_error("if_array() failed");
  }

  if (std::size(ja) == 0) {
    throw std::invalid_argument("metadata json file does not include archives");
  }
  return ja;
}

Metadata parse_metadata(const std::string& filename) {
  std::ifstream i(filename);
  if (!i.is_open()) {
    throw std::runtime_error(
        fmt::format("failed to open metadata json file: {}", filename));
  }
  std::string string_json((std::istreambuf_iterator<char>(i)),
                          std::istreambuf_iterator<char>());
  boost::json::error_code ec;
  boost::json::value jv = boost::json::parse(string_json, ec);
  if (ec) {
    throw std::runtime_error(fmt::format(
        "failed to parse metadata json file: message", ec.message()));
  }

  Metadata metadata(filename, jv);

  spdlog::debug("metadata min_start_time_offset={}",
                metadata.getMinStartTimeOffset());
  spdlog::debug("metadata max_start_time_offset={}",
                metadata.getMaxStopTimeOffset());
  for (const auto& archive : metadata.getArchives()) {
    spdlog::debug("  file_path='{} start_time_offset={} stop_time_offset={}",
                  archive.getPath().string(), archive.getStartTimeOffset(),
                  archive.getStopTimeOffset());
  }

  return metadata;
}

void Metadata::adjustTimeOffsets(double diff) {
  m_min_start_time_offset += diff;
  m_max_stop_time_offset += diff;
  for (auto& archive : m_archives) {
    archive.adjustTimeOffsets(diff);
  }
}

MetadataSet::MetadataSet(const Metadata& t_normal)
    : m_normal(t_normal), m_preferred({}) {}

void MetadataSet::setPrefered(const Metadata& t_preferred) {
  m_has_preferred = true;
  m_preferred = t_preferred;
  auto diff = m_normal.getCreatedAt() - m_preferred.getCreatedAt();
  if (diff > 0) {
    m_normal.adjustTimeOffsets(diff);
  } else {
    m_preferred.adjustTimeOffsets(diff);
  }
}

std::vector<Archive> MetadataSet::getArchives() const {
  if (m_has_preferred) {
    std::vector<hisui::Archive> archives;
    auto a0 = m_normal.getArchives();
    archives.insert(std::end(archives), std::begin(a0), std::end(a0));
    auto a1 = m_preferred.getArchives();
    archives.insert(std::end(archives), std::begin(a1), std::end(a1));
    return archives;
  }
  return m_normal.getArchives();
}

std::vector<Archive> MetadataSet::getNormalArchives() const {
  return m_normal.getArchives();
}

Metadata MetadataSet::getNormal() const {
  return m_normal;
}

Metadata MetadataSet::getPreferred() const {
  return m_preferred;
}

bool MetadataSet::hasPreferred() const {
  return m_has_preferred;
}

double MetadataSet::getMaxStopTimeOffset() const {
  if (m_has_preferred) {
    return std::max(m_normal.getMaxStopTimeOffset(),
                    m_preferred.getMaxStopTimeOffset());
  }
  return m_normal.getMaxStopTimeOffset();
}

boost::json::string Metadata::getRecordingID() const {
  return m_recording_id;
}

boost::json::string get_string_from_json_object(boost::json::object o,
                                                const std::string& key) {
  if (auto p = o[key].if_string()) {
    return *p;
  }
  throw std::runtime_error(fmt::format("o[{}].if_string() failed", key));
}

double get_double_from_json_object(boost::json::object o,
                                   const std::string& key) {
  if (o[key].is_number()) {
    boost::json::error_code ec;
    auto value = o[key].to_number<double>(ec);
    if (ec) {
      throw std::runtime_error(
          fmt::format("o[{}].to_number() failed: {}", key, ec.message()));
    }
    return value;
  }
  throw std::runtime_error("start_time_offset is not number");
}

}  // namespace hisui
