#include "layout/region.hpp"

#include <fmt/core.h>
#include <spdlog/spdlog.h>

#include <algorithm>
#include <stdexcept>

#include "constants.hpp"
#include "layout/overlap.hpp"
#include "layout/source.hpp"
#include "video/yuv.hpp"

namespace hisui::layout {

std::string Region::getName() const {
  return m_name;
}

RegionInformation Region::getInformation() const {
  return {.pos = m_pos, .resolution = m_resolution};
}

std::int32_t Region::getZPos() const {
  return m_z_pos;
}

void Region::validateAndAdjust(const RegionPrepareParameters& params) {
  if (params.resolution.width < m_pos.x) {
    throw std::out_of_range(
        fmt::format("The x_pos({}) is out of composition's width({})", m_pos.x,
                    params.resolution.width));
  }

  if (params.resolution.height < m_pos.y) {
    throw std::out_of_range(
        fmt::format("The y_pos({}) is out of composition's height({})", m_pos.y,
                    params.resolution.height));
  }

  if (m_z_pos > 99 || m_z_pos < -99) {
    throw std::out_of_range(
        fmt::format("The z_pos({}) is out of [-99, 99]", m_z_pos));
  }

  if (m_resolution.width != 0) {
    if (m_pos.x + m_resolution.width > params.resolution.width) {
      throw std::out_of_range(fmt::format(
          "The x_pos({}) & width({}) is out of composition's width({})",
          m_pos.x, m_resolution.width, params.resolution.width));
    }
  } else {
    m_resolution.width = params.resolution.width - m_pos.x;
  }

  if (m_resolution.height != 0) {
    if (m_pos.y + m_resolution.height > params.resolution.height) {
      throw std::out_of_range(fmt::format(
          "The y_pos({}) & height({}) is out of composition's height({})",
          m_pos.y, m_resolution.height, params.resolution.height));
    }
  } else {
    m_resolution.height = params.resolution.height - m_pos.y;
  }

  // TODO(haruyama): 2 の倍数でよさそうだが, 4 の倍数のほうがいいかも
  m_resolution.width = (m_resolution.width >> 1) << 1;
  m_resolution.height = (m_resolution.height >> 1) << 1;

  if (m_resolution.width < 16) {
    throw std::out_of_range(
        fmt::format("width({}) is too small", m_resolution.width));
  }
  if (m_resolution.height < 16) {
    throw std::out_of_range(
        fmt::format("height({}) is too small", m_resolution.height));
  }

  if (m_max_columns > 1000) {
    throw std::out_of_range(
        fmt::format("max_columns({})  is too large", m_max_columns));
  }
  if (m_max_rows > 1000) {
    throw std::out_of_range(
        fmt::format("max_rows({}) is too large", m_max_rows));
  }

  auto out_of_range_cells_excluded_iter =
      std::find_if(std::begin(m_cells_excluded), std::end(m_cells_excluded),
                   [](const auto e) { return e > 999999; });
  if (out_of_range_cells_excluded_iter != std::end(m_cells_excluded)) {
    throw std::out_of_range(
        fmt::format("cells_excluded contains too large value({})",
                    *out_of_range_cells_excluded_iter));
  }

  // https://s3.amazonaws.com/com.twilio.prod.twilio-docs/images/composer_understanding_trim.original.png
  // constrained_grid でかつ reuse: none の場合
  if (m_max_rows > 0 && m_max_columns > 0 && m_reuse == Reuse::None) {
    auto grid_size = m_max_rows * m_max_columns;
    if (std::size(m_video_source_filenames) > grid_size) {
      // grid に収まらないソースは破棄される
      spdlog::info(
          "region {}: constrained_grid & reuse=none: the size of "
          "video_source({}) is "
          "greater than grid size({})",
          m_name, std::size(m_video_source_filenames), grid_size);
      m_video_source_filenames.resize(grid_size);
      spdlog::info("region {}: video_sources resized", m_name);
    }
  }
}

const RegionPrepareResult Region::prepare(
    const RegionPrepareParameters& params) {
  validateAndAdjust(params);

  std::size_t index = 0;
  for (const auto& f : m_video_source_filenames) {
    try {
      auto archive = parse_archive(f);
      m_video_sources.push_back(
          std::make_shared<VideoSource>(archive->getSourceParameters(index++)));
    } catch (const std::exception& e) {
      spdlog::error("region {}: parsing video_source({}) failed: {}", m_name, f,
                    e.what());
      std::exit(EXIT_FAILURE);
    }
  }

  // 最大に overlap する video の数, trim 可能な interval, 終了時間を算出
  std::vector<Interval> source_intervals;
  std::transform(
      std::begin(m_video_sources), std::end(m_video_sources),
      std::back_inserter(source_intervals),
      [](const auto& s) -> Interval { return s->getSourceInterval(); });
  auto overlap_result =
      overlap_intervals({.intervals = source_intervals, .reuse = m_reuse});

  m_min_start_time = overlap_result.min_start_time;
  m_max_end_time = overlap_result.max_end_time;
  spdlog::debug("region {}: min_start: {}, max_end: {}", m_name,
                m_min_start_time, m_max_end_time);

  for (const auto& i : overlap_result.trim_intervals) {
    spdlog::debug("    trim_interval: [{}, {}]", i.start_time, i.end_time);
  }

  // cells_excluded を sort し unique に
  std::sort(std::begin(m_cells_excluded), std::end(m_cells_excluded));
  auto ret =
      std::unique(std::begin(m_cells_excluded), std::end(m_cells_excluded));
  m_cells_excluded.erase(ret, std::end(m_cells_excluded));

  // cells_excluded を考慮した最大の video cell の数を算出
  auto max_cells = add_number_of_excluded_cells({
      .number_of_sources = overlap_result.max_number_of_overlap,
      .cells_excluded = m_cells_excluded,
  });

  // grid の次元を算出
  m_grid_dimension = calc_grid_dimension({.max_columns = m_max_columns,
                                          .max_rows = m_max_rows,
                                          .number_of_sources = max_cells});
  auto cell_resolution_and_posiitons = calc_cell_resolution_and_positions({
      .grid_dimension = m_grid_dimension,
      .region_resolution = m_resolution,
      .is_width_frame_on_ends =
          !(m_resolution.width == params.resolution.width),
      .is_height_frame_on_ends =
          !(m_resolution.height == params.resolution.height),
  });

  // cell に情報を詰め m_cells に追加する
  for (std::size_t i = 0; i < m_grid_dimension.rows * m_grid_dimension.columns;
       ++i) {
    if (i >= max_cells) {
      break;
    }
    CellStatus status = CellStatus::Fresh;
    auto it =
        std::find(std::begin(m_cells_excluded), std::end(m_cells_excluded), i);
    if (it != std::end(m_cells_excluded)) {
      status = CellStatus::Excluded;
    }
    m_cells.push_back(std::make_shared<Cell>(
        CellParameters{.index = i,
                       .pos = cell_resolution_and_posiitons.positions[i],
                       .resolution = cell_resolution_and_posiitons.resolution,
                       .status = status,
                       .filter_mode = m_filter_mode}));
    auto info = m_cells[i]->getInformation();
    spdlog::debug("    cell[{}]: x: {}, y:{}, w:{}, h:{}", i, info.pos.x,
                  info.pos.y, info.resolution.width, info.resolution.height);
  }
  spdlog::debug("    cell size: {}", std::size(m_cells));

  // YUV のサイズ
  m_plane_sizes[0] = m_resolution.width * m_resolution.height;
  m_plane_sizes[1] = (m_plane_sizes[0] + 3) >> 2;
  m_plane_sizes[2] = m_plane_sizes[1];

  m_yuv_image = std::make_shared<hisui::video::YUVImage>(m_resolution.width,
                                                         m_resolution.height);

  // YUV のデフォルト値 (黒)
  m_plane_default_values[0] = 0;
  m_plane_default_values[1] = 128;
  m_plane_default_values[2] = 128;

  return {.trim_intervals = overlap_result.trim_intervals};
}

double Region::getMaxEndTime() const {
  return m_max_end_time;
}

void Region::substructTrimIntervals(const TrimIntervals& params) {
  for (auto s : m_video_sources) {
    s->substructTrimIntervals(params);
  }

  spdlog::debug("region {}: min_start: {}, max_end: {}", m_name,
                m_min_start_time, m_max_end_time);

  auto start_interval =
      substruct_trim_intervals({.interval = {0, m_min_start_time},
                                .trim_intervals = params.trim_intervals});
  m_min_start_time = start_interval.end_time;
  auto end_interval =
      substruct_trim_intervals({.interval = {0, m_max_end_time},
                                .trim_intervals = params.trim_intervals});
  m_max_end_time = end_interval.end_time;
  spdlog::debug("region {}: min_start: {}, max_end: {}", m_name,
                m_min_start_time, m_max_end_time);
}

// cells の中の cell に video_source を設定する (設定できない場合もある)
void set_video_source_to_cells(const SetVideoSourceToCells& params) {
  auto video_source = params.video_source;
  auto reuse = params.reuse;
  auto cells = params.cells;

  // spdlog::debug("show_newest: {} {} {}", reuse, video_source->index,
  //               video_source->encoding_interval.getUpper());

  // Fresh な cell があればその先頭を利用する
  auto it_index = std::find_if(
      std::begin(cells), std::end(cells), [&video_source](const auto& cell) {
        // return cell->hasVideoSourceConnectionID(video_source->connection_id);
        return cell->hasVideoSourceIndex(video_source->getIndex());
      });
  if (it_index != std::end(cells)) {
    return;
  }
  auto it_fresh = std::find_if(
      std::begin(cells), std::end(cells),
      [](const auto& cell) { return cell->hasStatus(CellStatus::Fresh); });
  if (it_fresh != std::end(cells)) {
    (*it_fresh)->setSource(video_source);
    return;
  }

  // Reuse が none なら終了
  if (reuse == Reuse::None) {
    return;
  }

  // Idle な cell があればその先頭を利用する
  auto it_idle = std::find_if(
      std::begin(cells), std::end(cells),
      [](const auto& cell) { return cell->hasStatus(CellStatus::Idle); });
  if (it_idle != std::end(cells)) {
    (*it_idle)->setSource(video_source);
    return;
  }

  // Reuse が show_oldest なら終了
  if (reuse == Reuse::ShowOldest) {
    return;
  }

  // 開始時間が video_source よりも前の Used Cell を取得する
  std::vector<std::shared_ptr<Cell>> candidates;
  std::copy_if(
      std::begin(cells), std::end(cells), std::back_inserter(candidates),
      [&video_source](const auto& cell) {
        return cell->getStartTime() < video_source->getMinEncodingTime();
      });

  auto size = std::size(candidates);
  if (size == 0) {
    return;
  } else if (size == 1) {
    candidates[0]->setSource(video_source);
    return;
  }

  // 終了時刻が最小の cell を選択する
  auto it_min = std::min_element(std::begin(candidates), std::end(candidates),
                                 [](const auto& a, const auto& b) {
                                   return a->getEndTime() < b->getEndTime();
                                 });

  if (it_min != std::end(cells)) {
    (*it_min)->setSource(video_source);
  }
}

Region::Region(const RegionParameters& params)
    : m_name(params.name),
      m_pos(params.pos),
      m_z_pos(params.z_pos),
      m_resolution(params.resolution),
      m_max_columns(params.max_columns),
      m_max_rows(params.max_rows),
      m_cells_excluded(params.cells_excluded),
      m_reuse(params.reuse),
      m_video_source_filenames(params.video_source_filenames),
      m_filter_mode(params.filter_mode) {}

void Region::dump() const {
  spdlog::debug("  name: {}", m_name);
  spdlog::debug("  position: x: {} y: {}", m_pos.x, m_pos.y);
  spdlog::debug("  z_position: {} ", m_z_pos);
  spdlog::debug("  cells_excluded: [{}]", fmt::join(m_cells_excluded, ", "));
  spdlog::debug("  resolution: {}x{}", m_resolution.width, m_resolution.height);
  spdlog::debug("  max_columns: {}", m_max_columns);
  spdlog::debug("  max_rows: {}", m_max_rows);
  spdlog::debug("  video_sources: [{}]",
                fmt::join(m_video_source_filenames, ", "));
  spdlog::debug("  reuse: {}", m_reuse == Reuse::None         ? "none"
                               : m_reuse == Reuse::ShowOldest ? "show_oldest"
                                                              : "show_newest");
  if (!std::empty(m_video_sources)) {
    spdlog::debug("  grid_dimension: {}x{}", m_grid_dimension.columns,
                  m_grid_dimension.rows);
    for (const auto& a : m_video_sources) {
      a->dump();
    }
  }
}

void Region::setEncodingInterval() {
  m_encoding_interval.set(
      static_cast<std::uint64_t>(
          std::floor(m_min_start_time *
                     static_cast<double>(hisui::Constants::NANO_SECOND))),
      static_cast<std::uint64_t>(
          std::ceil(m_max_end_time *
                    static_cast<double>(hisui::Constants::NANO_SECOND))));
  for (auto& s : m_video_sources) {
    s->setEncodingInterval(hisui::Constants::NANO_SECOND);
  }
}

RegionGetYUVResult Region::getYUV(const std::uint64_t t) {
  if (!m_encoding_interval.isIn(t)) {
    return {.is_rendered = false, .yuv = m_yuv_image};
  }

  reset_cells_source({.cells = m_cells, .time = t});

  for (const auto& video_source : m_video_sources) {
    if (video_source->isIn(t)) {
      set_video_source_to_cells(
          {.video_source = video_source, .reuse = m_reuse, .cells = m_cells});
    }
  }

  for (std::size_t p = 0; p < 3; ++p) {
    std::fill_n(m_yuv_image->yuv[p], m_plane_sizes[p],
                m_plane_default_values[p]);
  }

  for (auto& cell : m_cells) {
    if (cell->hasStatus(CellStatus::Used)) {
      auto yuv_image = cell->getYUV(t);
      auto info = cell->getInformation();
      for (std::size_t p = 0; p < 3; ++p) {
        if (p == 0) {
          hisui::video::overlay_yuv_planes(
              m_yuv_image->yuv[p], yuv_image->yuv[p], m_resolution.width,
              info.pos.x, info.pos.y, info.resolution.width,
              info.resolution.height);
        } else {
          hisui::video::overlay_yuv_planes(
              m_yuv_image->yuv[p], yuv_image->yuv[p], m_resolution.width >> 1,
              info.pos.x >> 1, info.pos.y >> 1, info.resolution.width >> 1,
              info.resolution.height >> 1);
        }
      }
    }
  }

  return {.is_rendered = true, .yuv = m_yuv_image};
}

}  // namespace hisui::layout
