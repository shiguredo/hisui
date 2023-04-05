#include "layout/archive.hpp"

#include <fmt/core.h>
#include <spdlog/spdlog.h>

#include <filesystem>
#include <fstream>
#include <memory>
#include <stdexcept>
#include <string>

#include <boost/json/array.hpp>
#include <boost/json/impl/array.hpp>
#include <boost/json/object.hpp>
#include <boost/json/parse.hpp>
#include <boost/json/string.hpp>
#include <boost/json/system_error.hpp>
#include <boost/json/value.hpp>

#include "layout/interval.hpp"
#include "util/file.hpp"
#include "util/json.hpp"

namespace hisui::layout {

std::shared_ptr<Archive> parse_archive(const std::string& filename) {
  auto json_path_result = hisui::util::find_file(filename);
  if (!json_path_result.found) {
    throw std::invalid_argument(json_path_result.message);
  }
  std::ifstream i(json_path_result.path);
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

  const auto current_path = std::filesystem::current_path();
  std::filesystem::current_path(json_path_result.path.parent_path());

  boost::json::object j;
  if (jv.is_object()) {
    j = jv.as_object();
  } else {
    throw std::runtime_error("jv is not object");
  }

  auto connection_id =
      hisui::util::get_string_from_json_object(j, "connection_id");
  auto start_time = hisui::util::get_double_from_json_object(j, "start_time");
  auto stop_time = hisui::util::get_double_from_json_object(j, "stop_time");

  // filename から webm ファイルの path を解決する
  auto filename_string =
      hisui::util::get_string_from_json_object(j, "filename");
  auto filename_result = hisui::util::find_file(std::string(filename_string));
  std::filesystem::path file_path;
  if (filename_result.found) {
    file_path = filename_result.path;
  } else {
    // filename でうまくいかなかったら file_path から webm ファイルの path を解決する
    auto file_path_string =
        hisui::util::get_string_from_json_object(j, "file_path");
    auto file_path_result =
        hisui::util::find_file(std::string(file_path_string));
    if (file_path_result.found) {
      file_path = file_path_result.path;
    } else {
      throw std::invalid_argument(
          fmt::format("filename() and file_path() do not exsit", filename,
                      file_path_string));
    }
  }

  std::filesystem::current_path(current_path);

  return std::make_shared<Archive>(
      ArchiveParameters{.path = json_path_result.path,
                        .file_path = file_path,
                        .connection_id = std::string(connection_id),
                        .start_time = start_time,
                        .stop_time = stop_time});
}

Archive::Archive(const ArchiveParameters& params)
    : m_path(params.path),
      m_file_path(params.file_path),
      m_connection_id(params.connection_id),
      m_start_time(params.start_time),
      m_stop_time(params.stop_time) {}

void Archive::dump() const {
  spdlog::debug("path: {}", m_path.string());
  spdlog::debug("file_path: {}", m_file_path.string());
  spdlog::debug("connection_id: {}", m_connection_id);
  spdlog::debug("start_time: {}", m_start_time);
  spdlog::debug("stop_time: {}", m_stop_time);
}

const SourceParameters Archive::getSourceParameters(
    const std::size_t index) const {
  return SourceParameters{
      .file_path = m_file_path,
      .index = index,
      .connection_id = m_connection_id,
      .start_time = m_start_time,
      .end_time = m_stop_time,
  };
}

void Archive::substructTrimIntervals(const TrimIntervals& params) {
  auto interval = substruct_trim_intervals(
      {.interval = getInterval(), .trim_intervals = params.trim_intervals});
  m_start_time = interval.start_time;
  m_stop_time = interval.end_time;
}

Interval Archive::getInterval() const {
  return {.start_time = m_start_time, .end_time = m_stop_time};
}

hisui::ArchiveItem Archive::getArchiveItem() const {
  return hisui::ArchiveItem(m_file_path, m_connection_id, m_start_time,
                            m_stop_time);
}

}  // namespace hisui::layout
