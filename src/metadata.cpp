#include "metadata.hpp"

#include <bits/exception.h>
#include <spdlog/fmt/bundled/format.h>
#include <spdlog/spdlog.h>

#include <algorithm>
#include <compare>  // NOLINT
#include <filesystem>
#include <fstream>
#include <iterator>
#include <stdexcept>
#include <tuple>
#include <utility>
#include <vector>

#include <boost/json/array.hpp>
#include <boost/json/impl/array.hpp>
#include <boost/json/object.hpp>
#include <boost/json/parse.hpp>
#include <boost/json/string.hpp>
#include <boost/json/system_error.hpp>
#include <boost/json/value.hpp>

#include "archive_item.hpp"
#include "util/json.hpp"

namespace hisui {

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
    auto a_file_path = hisui::util::get_string_from_json_object(o, "file_path");
    auto a_connection_id =
        hisui::util::get_string_from_json_object(o, "connection_id");
    double a_start_time_offset =
        hisui::util::get_double_from_json_object(o, "start_time_offset");
    double a_stop_time_offset =
        hisui::util::get_double_from_json_object(o, "stop_time_offset");
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
    ArchiveItem archive(path, get<1>(a), get<2>(a), get<3>(a));
    m_archives.push_back(archive);
  }
  setTimeOffsets();
  std::filesystem::current_path(current_path);
}

Metadata::Metadata(const std::vector<ArchiveItem>& t_archives)
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

std::vector<ArchiveItem> Metadata::getArchiveItems() const {
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

  m_recording_id = hisui::util::get_string_from_json_object(j, "recording_id");
  m_created_at = hisui::util::get_double_from_json_object(j, "created_at");

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

boost::json::string Metadata::getRecordingID() const {
  return m_recording_id;
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
        "failed to parse metadata json file: message: {}", ec.message()));
  }

  Metadata metadata(filename, jv);

  spdlog::debug("metadata min_start_time_offset={}",
                metadata.getMinStartTimeOffset());
  spdlog::debug("metadata max_start_time_offset={}",
                metadata.getMaxStopTimeOffset());
  for (const auto& archive : metadata.getArchiveItems()) {
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
    m_preferred.adjustTimeOffsets(-diff);
  }
}

std::vector<ArchiveItem> MetadataSet::getArchiveItems() const {
  if (m_has_preferred) {
    std::vector<hisui::ArchiveItem> archives;
    auto a0 = m_normal.getArchiveItems();
    archives.insert(std::end(archives), std::begin(a0), std::end(a0));
    auto a1 = m_preferred.getArchiveItems();
    archives.insert(std::end(archives), std::begin(a1), std::end(a1));
    return archives;
  }
  return m_normal.getArchiveItems();
}

std::vector<ArchiveItem> MetadataSet::getNormalArchives() const {
  return m_normal.getArchiveItems();
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

void MetadataSet::split(const std::string& connection_id) {
  m_preferred.copyWithoutArchives(m_normal);
  const auto archives = m_normal.deleteArchivesByConnectionID(connection_id);
  if (archives.empty()) {
    throw std::runtime_error(
        fmt::format("connection_id {} is not found", connection_id));
  }

  m_preferred.setArchives(archives);
  m_has_preferred = true;
}

void Metadata::copyWithoutArchives(const Metadata& orig) {
  m_path = orig.getPath();
  m_min_start_time_offset = orig.getMinStartTimeOffset();
  m_max_stop_time_offset = orig.getMaxStopTimeOffset();
  m_created_at = orig.getCreatedAt();
  m_recording_id = orig.m_recording_id;
}

std::vector<ArchiveItem> Metadata::deleteArchivesByConnectionID(
    const std::string& connection_id) {
  std::vector<ArchiveItem> undeleted{};
  std::vector<ArchiveItem> deleted{};

  for (const auto& archive : m_archives) {
    if (archive.getConnectionID() == connection_id) {
      deleted.push_back(archive);
    } else {
      undeleted.push_back(archive);
    }
  }
  m_archives = undeleted;
  spdlog::debug("undeleted: {}, deleted: {}", undeleted.size(), deleted.size());
  return deleted;
}

void Metadata::setArchives(const std::vector<ArchiveItem>& t_archives) {
  m_archives = t_archives;
}

std::filesystem::path Metadata::getPath() const {
  return m_path;
}

}  // namespace hisui
