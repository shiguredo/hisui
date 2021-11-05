#include "layout/cell.hpp"

#include <libyuv/scale.h>

#include <limits>
#include <stdexcept>

namespace hisui::layout {

Cell::Cell(const CellParameters& params)
    : m_index(params.index),
      m_pos(params.pos),
      m_resolution(params.resolution),
      m_status(params.status) {
  m_end_time = std::numeric_limits<std::uint64_t>::max();
  if (m_status != CellStatus::Excluded) {
    // TODO(haruyama): filtermode の指定
    m_scaler = std::make_shared<hisui::video::PreserveAspectRatioScaler>(
        m_resolution.width, m_resolution.height, libyuv::kFilterBox);
  }
}

const std::shared_ptr<hisui::video::YUVImage> Cell::getYUV(
    const std::uint64_t t) {
  return m_scaler->scale(m_source->source->getYUV(
      m_source->encoding_interval.getSubstructLower(t)));
}

bool Cell::hasVideoSourceConnectionID(const std::string& connection_id) {
  return m_source && m_source->connection_id == connection_id;
}

bool Cell::hasVideoSourceIndex(const size_t index) {
  return m_source && m_source->index == index;
}

bool Cell::hasStatus(const CellStatus status) {
  return m_status == status;
}

void Cell::setSource(std::shared_ptr<VideoSource> source) {
  m_status = CellStatus::Used;
  m_source = source;
  m_end_time = source->encoding_interval.getUpper();
}

void Cell::resetSource(const std::uint64_t time) {
  if (time >= m_end_time) {
    m_status = CellStatus::Idle;
    m_source = nullptr;
    m_end_time = std::numeric_limits<std::uint64_t>::max();
  }
}

std::uint64_t Cell::getEndTime() const {
  return m_end_time;
}

void Cell::setExcludedStatus() {
  m_status = CellStatus::Excluded;
}

void reset_cells_source(const ResetCellsSource& params) {
  for (auto cell : params.cells) {
    cell->resetSource(params.time);
  }
}

const CellInformation Cell::getInformation() const {
  return {.pos = m_pos, .resolution = m_resolution};
}

}  // namespace hisui::layout
