#include "layout/metadata.hpp"

#include <spdlog/spdlog.h>

#include <algorithm>
#include <cstdlib>
#include <fstream>
#include <list>
#include <regex>
#include <stdexcept>
#include <utility>

#include <boost/json/array.hpp>
#include <boost/json/impl/array.hpp>
#include <boost/json/object.hpp>
#include <boost/json/parse.hpp>
#include <boost/json/string.hpp>
#include <boost/json/system_error.hpp>
#include <boost/json/value.hpp>
#include <boost/json/value_to.hpp>

#include "layout/archive.hpp"
#include "layout/overlap.hpp"
#include "layout/source.hpp"
#include "util/file.hpp"
#include "util/json.hpp"
#include "util/wildcard.hpp"

namespace hisui::layout {

void Metadata::parseVideoLayout(
    boost::json::object j,
    const std::vector<std::string>& fixed_excluded_patterns) {
  auto key = "video_layout";
  if (!j.contains(key) || j[key].is_null()) {
    return;
  }
  auto vl = j[key].as_object();
  for (const auto& region : vl) {
    std::string name(region.key());
    if (region.value().is_object()) {
      try {
        m_regions.push_back(parseRegion(name, region.value().as_object(),
                                        fixed_excluded_patterns));
      } catch (const std::exception& e) {
        spdlog::error("parsing region '{}' failed: {}", name, e.what());
        std::exit(EXIT_FAILURE);
      }
    } else {
      throw std::invalid_argument(
          fmt::format("region: {} is not object", name));
    }
  }
}

void Metadata::dump() const {
  spdlog::debug("format: {}",
                m_format == hisui::config::OutContainer::MP4 ? "mp4" : "webm");
  spdlog::debug("bitrate: {}", m_bitrate);
  spdlog::debug("resolution: {}x{}", m_resolution.width, m_resolution.height);
  spdlog::debug("trim: {}", m_trim);
  spdlog::debug("audio_sources: [{}]",
                fmt::join(m_audio_source_filenames, ", "));
  spdlog::debug("video_layout");
  for (const auto& region : m_regions) {
    region->dump();
    spdlog::debug("");
  }
  if (!std::empty(m_audio_archives)) {
    for (const auto& a : m_audio_archives) {
      a->dump();
    }
    spdlog::debug("audio_max_end_time: {}", m_audio_max_end_time);
    spdlog::debug("max_end_time: {}", m_max_end_time);
  }
}

Metadata::Metadata(const std::string& file_path,
                   const boost::json::value& jv,
                   const hisui::Config& config)
    : m_path(file_path), m_filter_mode(config.libyuv_filter_mode) {
  if (m_path.is_relative()) {
    m_path = std::filesystem::absolute(m_path);
  }
  m_working_path = std::filesystem::absolute(std::filesystem::current_path());
  std::filesystem::current_path(m_path.parent_path());

  boost::json::object j;
  if (jv.is_object()) {
    j = jv.as_object();
  } else {
    throw std::runtime_error("jv is not object");
  }

  m_bitrate = static_cast<std::uint64_t>(
      hisui::util::get_double_from_json_object_with_default(j, "bitrate", 0));
  auto format = hisui::util::get_string_from_json_object_with_default(
      j, "format", "webm");
  if (format == "mp4") {
    m_format = hisui::config::OutContainer::MP4;
  } else if (format == "webm") {
    m_format = hisui::config::OutContainer::WebM;
  } else {
    throw std::invalid_argument(fmt::format("format is invalid: {}", format));
  }

  std::string resolution(
      hisui::util::get_string_from_json_object(j, "resolution"));

  std::smatch m;
  if (std::regex_match(resolution, m, std::regex(R"((\d+)x(\d+))"))) {
    m_resolution.width = static_cast<std::uint32_t>(std::stoul(m[1].str()));
    m_resolution.height = static_cast<std::uint32_t>(std::stoul(m[2].str()));
  } else {
    throw std::invalid_argument(
        fmt::format("resolution is invalid: {}", resolution));
  }
  m_trim = hisui::util::get_bool_from_json_object_with_default(j, "trim", true);

  auto audio_sources = hisui::util::get_array_from_json_object_with_default(
      j, "audio_sources", boost::json::array());

  std::vector<std::string> audio_source_filenames;
  for (const auto& v : audio_sources) {
    if (v.is_string()) {
      auto pattern = std::string(v.as_string());
      auto filenames = hisui::util::glob(pattern);
      if (std::empty(filenames)) {
        throw std::invalid_argument(fmt::format(
            "pattern '{}' in audio_sources is not matched with filenames",
            pattern));
      }
      audio_source_filenames.insert(std::end(audio_source_filenames),
                                    std::begin(filenames), std::end(filenames));
    } else {
      throw std::invalid_argument(
          fmt::format("{} contains a non-string value", "audio_sources"));
    }
  }

  auto audio_sources_excluded =
      hisui::util::get_array_from_json_object_with_default(
          j, "audio_sources_excluded", boost::json::array());

  for (const auto& v : audio_sources_excluded) {
    if (v.is_string()) {
      auto pattern = std::string(v.as_string());
      auto result = std::remove_if(std::begin(audio_source_filenames),
                                   std::end(audio_source_filenames),
                                   [&pattern](const auto& text) {
                                     return hisui::util::wildcard_match(
                                         {.text = text, .pattern = pattern});
                                   });
      audio_source_filenames.erase(result, std::end(audio_source_filenames));
    } else {
      throw std::invalid_argument(fmt::format("{} contains a non-string value",
                                              "audio_sources_excluded"));
    }
  }

  std::vector<std::string> fixed_excluded_patterns = {
      m_path.filename(), "report-*.json", "*.webm"};

  for (const auto& pattern : fixed_excluded_patterns) {
    auto result = std::remove_if(std::begin(audio_source_filenames),
                                 std::end(audio_source_filenames),
                                 [&pattern](const auto& text) {
                                   return hisui::util::wildcard_match(
                                       {.text = text, .pattern = pattern});
                                 });
    audio_source_filenames.erase(result, std::end(audio_source_filenames));
  }

  m_audio_source_filenames = audio_source_filenames;
  parseVideoLayout(j, fixed_excluded_patterns);
}

Metadata parse_metadata(const hisui::Config& config) {
  try {
    auto filename = config.layout;
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

    Metadata metadata(filename, jv, config);

    spdlog::debug("not prepared");

    metadata.prepare();

    metadata.dump();

    spdlog::debug("prepared");

    metadata.resetPath();

    return metadata;
  } catch (const std::exception& e) {
    spdlog::error("parsing layout metadata failed: {}", e.what());
    std::exit(EXIT_FAILURE);
  }
}

void Metadata::resetPath() const {
  std::filesystem::current_path(m_working_path);
}

void Metadata::prepare() {
  // 解像度は 4の倍数にまるめる
  // TODO(haruyama): 2 の倍数でいいかもしれない
  m_resolution.width = (m_resolution.width >> 2) << 2;
  m_resolution.height = (m_resolution.height >> 2) << 2;
  if (m_resolution.width < 16) {
    throw std::out_of_range(
        fmt::format("resolution.width({}) is too small", m_resolution.width));
  } else if (m_resolution.width > 3840) {
    throw std::out_of_range(
        fmt::format("resolution.width({}) is too large", m_resolution.width));
  }
  if (m_resolution.height < 16) {
    throw std::out_of_range(
        fmt::format("resolution.height({}) is too small", m_resolution.height));
  } else if (m_resolution.height > 3840) {
    throw std::out_of_range(
        fmt::format("resolution.height({}) is too large", m_resolution.height));
  }

  if (m_bitrate == 0) {
    // TODO(haruyama): bitrate の初期値
    m_bitrate = m_resolution.width * m_resolution.height / 300;
    spdlog::info("bitrate==0. set {} to bitrate()", m_bitrate);
  }
  if (m_bitrate < 100) {
    spdlog::info("bitrate({}) is small. set 100 to bitrate", m_bitrate);
    m_bitrate = 100;
  }

  spdlog::debug("processing audio");

  std::list<std::vector<Interval>> list_of_trim_intervals;
  for (const auto& f : m_audio_source_filenames) {
    try {
      auto archive = parse_archive(f);
      m_audio_archives.push_back(archive);
    } catch (const std::exception& e) {
      spdlog::error("parsing audio_source({}) failed: {}", f, e.what());
      std::exit(EXIT_FAILURE);
    }
  }

  // trim 可能な間隔を audio, video(regions) からそれぞれ算出
  std::vector<Interval> audio_source_intervals;
  std::transform(std::begin(m_audio_archives), std::end(m_audio_archives),
                 std::back_inserter(audio_source_intervals),
                 [](const auto& a) -> Interval { return a->getInterval(); });
  auto audio_overlap_result = overlap_intervals(
      {.intervals = audio_source_intervals, .reuse = Reuse::None});

  list_of_trim_intervals.push_back(audio_overlap_result.trim_intervals);

  spdlog::debug("processing region");

  for (const auto& region : m_regions) {
    try {
      auto result = region->prepare({.resolution = m_resolution});
      list_of_trim_intervals.push_back(result.trim_intervals);
    } catch (const std::exception& e) {
      spdlog::error("preparing region '{}' failed: {}", region->getName(),
                    e.what());
      std::exit(EXIT_FAILURE);
    }
  }

  // すべての trim 可能な間隔の中で重なっている部分を算出
  auto overlap_trim_intervals_result = overlap_trim_intervals(
      {.list_of_trim_intervals = list_of_trim_intervals});

  for (const auto& i : overlap_trim_intervals_result.trim_intervals) {
    spdlog::debug("    final trim_interval: [{}, {}]", i.start_time,
                  i.end_time);
  }

  std::vector<Interval> trim_intervals;
  // trim = true ならばすべての trim 可能間隔を削除する
  // trim = false ならば 0 で始まる trim 可能間隔を削除する
  if (m_trim) {
    trim_intervals = overlap_trim_intervals_result.trim_intervals;
  } else {
    if (!std::empty(overlap_trim_intervals_result.trim_intervals)) {
      if (overlap_trim_intervals_result.trim_intervals[0].start_time == 0) {
        trim_intervals.push_back(
            overlap_trim_intervals_result.trim_intervals[0]);
      }
    }
  }

  for (auto& a : m_audio_archives) {
    a->substructTrimIntervals({.trim_intervals = trim_intervals});
  }

  std::transform(
      std::begin(m_audio_archives), std::end(m_audio_archives),
      std::back_inserter(m_audio_archive_items),
      [](const auto& a) -> hisui::ArchiveItem { return a->getArchiveItem(); });

  auto interval = substruct_trim_intervals(
      {.interval = {0, audio_overlap_result.max_end_time},
       .trim_intervals = trim_intervals});
  m_audio_max_end_time = interval.end_time;
  m_max_end_time = interval.end_time;

  for (auto& r : m_regions) {
    r->substructTrimIntervals({.trim_intervals = trim_intervals});
    m_max_end_time = std::max(m_max_end_time, r->getMaxEndTime());
  }

  // m_regions は z_pos でソート
  std::sort(
      std::begin(m_regions), std::end(m_regions),
      [](const auto& a, const auto& b) { return a->getZPos() < b->getZPos(); });
}

std::shared_ptr<Region> Metadata::parseRegion(
    const std::string& name,
    boost::json::object jo,
    const std::vector<std::string>& fixed_excluded_patterns) {
  auto cells_excluded_array =
      hisui::util::get_array_from_json_object_with_default(
          jo, "cells_excluded", boost::json::array());
  std::vector<std::uint64_t> cells_excluded;
  for (const auto& v : cells_excluded_array) {
    if (v.is_number()) {
      boost::json::error_code ec;
      auto value = v.to_number<std::uint64_t>(ec);
      if (ec) {
        throw std::runtime_error(fmt::format(
            "cells_excluded: v.to_number<std::uint64_t>() failed: {}",
            ec.message()));
      }
      cells_excluded.push_back(value);
    } else {
      throw std::invalid_argument(
          fmt::format("{} contains a non-uint64 value", "cells_excluded"));
    }
  }

  auto video_sources = hisui::util::get_array_from_json_object_with_default(
      jo, "video_sources", boost::json::array());
  std::vector<std::string> video_source_filenames;

  for (const auto& v : video_sources) {
    if (v.is_string()) {
      auto pattern = std::string(v.as_string());
      auto filenames = hisui::util::glob(pattern);
      if (std::empty(filenames)) {
        throw std::invalid_argument(
            fmt::format("pattern '{}' is not matched with filenames", pattern));
      }
      video_source_filenames.insert(std::end(video_source_filenames),
                                    std::begin(filenames), std::end(filenames));
    } else {
      throw std::invalid_argument(
          fmt::format("{} contains a non-string value", "video_sources"));
    }
  }

  auto video_sources_excluded =
      hisui::util::get_array_from_json_object_with_default(
          jo, "video_sources_excluded", boost::json::array());

  for (const auto& v : video_sources_excluded) {
    if (v.is_string()) {
      auto pattern = std::string(v.as_string());
      auto result = std::remove_if(std::begin(video_source_filenames),
                                   std::end(video_source_filenames),
                                   [&pattern](const auto& text) {
                                     return hisui::util::wildcard_match(
                                         {.text = text, .pattern = pattern});
                                   });
      video_source_filenames.erase(result, std::end(video_source_filenames));
    } else {
      throw std::invalid_argument(fmt::format("{} contains a non-string value",
                                              "video_sources_excluded"));
    }
  }

  for (const auto& pattern : fixed_excluded_patterns) {
    auto result = std::remove_if(std::begin(video_source_filenames),
                                 std::end(video_source_filenames),
                                 [&pattern](const auto& text) {
                                   return hisui::util::wildcard_match(
                                       {.text = text, .pattern = pattern});
                                 });
    video_source_filenames.erase(result, std::end(video_source_filenames));
  }

  auto reuse_string = hisui::util::get_string_from_json_object_with_default(
      jo, "reuse", "show_oldest");
  Reuse reuse;
  if (reuse_string == "none") {
    reuse = Reuse::None;
  } else if (reuse_string == "show_oldest") {
    reuse = Reuse::ShowOldest;
  } else if (reuse_string == "show_newest") {
    reuse = Reuse::ShowNewest;
  } else {
    throw std::invalid_argument(
        fmt::format("reuse is invalid: {}", reuse_string));
  }

  RegionParameters params{
      .name = name,
      .pos{.x = static_cast<std::uint32_t>(
               hisui::util::get_double_from_json_object_with_default(
                   jo, "x_pos", 0)),
           .y = static_cast<std::uint32_t>(
               hisui::util::get_double_from_json_object_with_default(
                   jo, "y_pos", 0))},
      .z_pos = static_cast<std::int32_t>(
          hisui::util::get_double_from_json_object_with_default(jo, "z_pos",
                                                                0)),
      .resolution{.width = static_cast<std::uint32_t>(
                      hisui::util::get_double_from_json_object_with_default(
                          jo, "width", 0)),
                  .height = static_cast<std::uint32_t>(
                      hisui::util::get_double_from_json_object_with_default(
                          jo, "height", 0))},
      .max_columns = static_cast<std::uint32_t>(
          hisui::util::get_double_from_json_object_with_default(
              jo, "max_columns", 0)),
      .max_rows = static_cast<std::uint32_t>(
          hisui::util::get_double_from_json_object_with_default(jo, "max_rows",
                                                                0)),
      .cells_excluded = cells_excluded,
      .reuse = reuse,
      .video_source_filenames = video_source_filenames,
      .filter_mode = m_filter_mode,
  };

  return std::make_shared<Region>(params);
}

void Metadata::copyToConfig(hisui::Config* config) const {
  // TODO(haruyama): bitrate で audio も考慮する?
  config->out_video_bit_rate = static_cast<std::uint32_t>(m_bitrate);
  config->out_container = m_format;
  config->in_metadata_filename = m_path.string();
}

double Metadata::getMaxEndTime() const {
  return m_max_end_time;
}

std::vector<hisui::ArchiveItem> Metadata::getAudioArchiveItems() const {
  return m_audio_archive_items;
}

Resolution Metadata::getResolution() const {
  return m_resolution;
}

std::vector<std::shared_ptr<Region>> Metadata::getRegions() const {
  return m_regions;
}

}  // namespace hisui::layout
