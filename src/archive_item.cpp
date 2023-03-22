#include "archive_item.hpp"

#include <filesystem>
#include <string>

namespace hisui {

ArchiveItem::ArchiveItem(const std::filesystem::path& t_path,
                         const std::string& m_connection_id,
                         const double t_start_time_offset,
                         const double t_stop_time_offset)
    : m_path(t_path),
      m_connection_id(m_connection_id),
      m_start_time_offset(t_start_time_offset),
      m_stop_time_offset(t_stop_time_offset) {}

std::filesystem::path ArchiveItem::getPath() const {
  return m_path;
}

std::string ArchiveItem::getConnectionID() const {
  return m_connection_id;
}

double ArchiveItem::getStartTimeOffset() const {
  return m_start_time_offset;
}

double ArchiveItem::getStopTimeOffset() const {
  return m_stop_time_offset;
}

void ArchiveItem::adjustTimeOffsets(double diff) {
  m_start_time_offset += diff;
  m_stop_time_offset += diff;
}

}  // namespace hisui
