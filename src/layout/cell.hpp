#pragma once

#include <libyuv/scale.h>

#include <cstdint>
#include <memory>
#include <string>
#include <vector>

#include "layout/cell_util.hpp"
#include "layout/grid.hpp"
#include "layout/video_source.hpp"
#include "video/preserve_aspect_ratio_scaler.hpp"
#include "video/yuv.hpp"

namespace hisui::layout {

enum CellStatus {
  Fresh,
  Used,
  Idle,
  Excluded,
};

struct CellParameters {
  const std::uint64_t index;
  const Position& pos;
  const Resolution& resolution;
  const CellStatus status = CellStatus::Fresh;
  const libyuv::FilterMode filter_mode = libyuv::kFilterBox;
};

struct CellInformation {
  const Position& pos;
  const Resolution& resolution;
};

class Cell {
 public:
  explicit Cell(const CellParameters&);
  bool hasVideoSourceIndex(const std::size_t);
  bool hasVideoSourceConnectionID(const std::string&);
  bool hasStatus(const CellStatus);
  void setExcludedStatus();
  void setSource(std::shared_ptr<VideoSource>);
  void resetSource(const std::uint64_t);
  std::uint64_t getStartTime() const;
  std::uint64_t getEndTime() const;
  const std::shared_ptr<hisui::video::YUVImage> getYUV(const std::uint64_t);
  const CellInformation getInformation() const;

 private:
  std::uint64_t m_index;
  Position m_pos;
  Resolution m_resolution;
  CellStatus m_status;
  std::shared_ptr<VideoSource> m_source;
  std::uint64_t m_start_time = 0;
  std::uint64_t m_end_time;

  // std::shared_ptr<hisui::video::YUVImage> m_scaled_image;
  std::shared_ptr<hisui::video::PreserveAspectRatioScaler> m_scaler;
};

struct ResetCellsSource {
  const std::vector<std::shared_ptr<Cell>>& cells;
  const std::uint64_t time;
};

void reset_cells_source(const ResetCellsSource&);

}  // namespace hisui::layout
