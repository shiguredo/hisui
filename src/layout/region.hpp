#pragma once

#include <libyuv/scale.h>

#include <cstdint>
#include <memory>
#include <string>
#include <utility>
#include <vector>

#include "layout/archive.hpp"
#include "layout/cell.hpp"
#include "layout/grid.hpp"
#include "layout/reuse.hpp"
#include "layout/source.hpp"
#include "layout/video_source.hpp"
#include "video/yuv.hpp"

namespace hisui::layout {

struct RegionInformation {
  const Position& pos;
  const Resolution& resolution;
};

struct RegionParameters {
  const std::string name;
  const Position pos;
  const std::int32_t z_pos;
  const Resolution resolution;
  const std::uint32_t max_columns;
  const std::uint32_t max_rows;
  const std::vector<std::uint64_t>& cells_excluded = {};
  const Reuse reuse;
  const std::vector<std::string>& video_source_filenames = {};
  const libyuv::FilterMode filter_mode = libyuv::kFilterBox;
};

struct RegionPrepareParameters {
  const Resolution& resolution;
};

struct RegionPrepareResult {
  std::vector<Interval> trim_intervals;
};

struct RegionGetYUVResult {
  const bool is_rendered;
  const std::shared_ptr<hisui::video::YUVImage> yuv;
};

class Region {
 public:
  explicit Region(const RegionParameters&);

  void dump() const;
  std::string getName() const;
  RegionInformation getInformation() const;
  std::int32_t getZPos() const;
  const RegionPrepareResult prepare(const RegionPrepareParameters&);
  void substructTrimIntervals(const TrimIntervals&);
  double getMaxEndTime() const;
  void setEncodingInterval();
  RegionGetYUVResult getYUV(const std::uint64_t);

 private:
  std::string m_name;
  Position m_pos;
  std::int32_t m_z_pos;
  Resolution m_resolution;
  std::uint32_t m_max_columns;
  std::uint32_t m_max_rows;
  std::vector<std::uint64_t> m_cells_excluded;
  Reuse m_reuse;
  std::vector<std::string> m_video_source_filenames;
  libyuv::FilterMode m_filter_mode;

  // computed
  GridDimension m_grid_dimension;
  std::vector<std::shared_ptr<VideoSource>> m_video_sources;
  std::vector<std::shared_ptr<Cell>> m_cells;
  double m_min_start_time;
  double m_max_end_time;
  hisui::util::Interval m_encoding_interval{0, 0};

  std::shared_ptr<hisui::video::YUVImage> m_yuv_image;
  std::array<std::size_t, 3> m_plane_sizes;
  std::array<unsigned char, 3> m_plane_default_values;

  void validateAndAdjust(const RegionPrepareParameters&);
};

struct SetVideoSourceToCells {
  const std::shared_ptr<VideoSource>& video_source;
  Reuse reuse;
  const std::vector<std::shared_ptr<Cell>>& cells;
};

void set_video_source_to_cells(const SetVideoSourceToCells&);

}  // namespace hisui::layout
