#pragma once

#include <libyuv/scale.h>

#include <filesystem>
#include <memory>
#include <string>
#include <vector>

#include <boost/json/array.hpp>
#include <boost/json/impl/array.hpp>
#include <boost/json/object.hpp>
#include <boost/json/parse.hpp>
#include <boost/json/string.hpp>
#include <boost/json/system_error.hpp>
#include <boost/json/value.hpp>

#include "config.hpp"
#include "layout/archive.hpp"
#include "layout/cell_util.hpp"
#include "layout/region.hpp"

namespace hisui::layout {

class Metadata {
 public:
  Metadata(const std::string&, const boost::json::value&, const hisui::Config&);
  void dump() const;
  void prepare();
  void copyToConfig(hisui::Config*) const;
  double getMaxEndTime() const;
  std::vector<std::shared_ptr<Region>> getRegions() const;
  Resolution getResolution() const;
  void resetPath() const;

  std::vector<hisui::ArchiveItem> getAudioArchiveItems() const;
  double getMaxStopTimeOffset() const;

 private:
  std::filesystem::path m_path;

  std::vector<std::string> m_audio_source_filenames;
  std::uint64_t m_bitrate;
  hisui::config::OutContainer m_format;
  Resolution m_resolution;
  bool m_trim;
  std::filesystem::path m_working_path;

  std::vector<std::shared_ptr<Archive>> m_audio_archives;
  std::vector<hisui::ArchiveItem> m_audio_archive_items;
  double m_audio_max_end_time;
  double m_max_end_time;
  std::vector<std::shared_ptr<Region>> m_regions;
  libyuv::FilterMode m_filter_mode;

  void parseVideoLayout(
      boost::json::object j,
      const std::vector<std::string>& fixed_excluded_patterns);
  std::shared_ptr<Region> parseRegion(
      const std::string& name,
      boost::json::object jo,
      const std::vector<std::string>& fixed_excluded_patterns);
};

Metadata parse_metadata(const hisui::Config&);

}  // namespace hisui::layout
