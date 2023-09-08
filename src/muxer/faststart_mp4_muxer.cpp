#include "muxer/faststart_mp4_muxer.hpp"

#include <bits/exception.h>
#include <spdlog/fmt/bundled/format.h>
#include <spdlog/fmt/fmt.h>
#include <spdlog/spdlog.h>

#include <filesystem>
#include <iosfwd>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

#include "config.hpp"
#include "metadata.hpp"
#include "shiguredo/mp4/track/soun.hpp"
#include "shiguredo/mp4/track/vide.hpp"
#include "shiguredo/mp4/writer/faststart_writer.hpp"

namespace shiguredo::mp4::track {

class Track;

}

namespace hisui::muxer {

FaststartMP4Muxer::FaststartMP4Muxer(const hisui::Config& t_config,
                                     const MP4MuxerParameters& params)
    : MP4Muxer(params), m_config(t_config) {}

FaststartMP4Muxer::FaststartMP4Muxer(const hisui::Config& t_config,
                                     const MP4MuxerParametersForLayout& params)
    : MP4Muxer(params), m_config(t_config) {}

void FaststartMP4Muxer::setUp() {
  std::filesystem::path directory_for_faststart_intermediate_file;

  if (m_config.directory_for_faststart_intermediate_file != "") {
    directory_for_faststart_intermediate_file =
        m_config.directory_for_faststart_intermediate_file;
    if (!std::filesystem::is_directory(
            directory_for_faststart_intermediate_file)) {
      throw std::invalid_argument(
          fmt::format("{} is not directory",
                      m_config.directory_for_faststart_intermediate_file));
    }
  } else {
    std::filesystem::path metadata_path(m_config.in_metadata_filename);
    spdlog::debug("metadata_path: {}", metadata_path.string());

    if (metadata_path.is_relative()) {
      metadata_path = std::filesystem::absolute(metadata_path);
    }
    directory_for_faststart_intermediate_file = metadata_path.parent_path();
  }
  spdlog::debug("directory_for_faststart_intermediate_file: {}",
                directory_for_faststart_intermediate_file.string());

  if (m_config.out_video_codec == hisui::config::OutVideoCodec::H264) {
    m_faststart_writer =
        std::make_shared<shiguredo::mp4::writer::FaststartWriter>(
            m_ofs,
            shiguredo::mp4::writer::FaststartWriterParameters{
                .mvhd_timescale = 1000,
                .duration = static_cast<float>(m_duration),
                .mdat_path_templete =
                    directory_for_faststart_intermediate_file.string() +
                    std::filesystem::path::preferred_separator + "mdatXXXXXX",
                .ftyp_params = shiguredo::mp4::box::FtypParameters{
                    .major_brand = {'i', 's', 'o', 'm'},
                    .minor_version = 0,
                    .compatible_brands = {
                        {'m', 'p', '4', '1'},
                    }}});
  } else {
    m_faststart_writer =
        std::make_shared<shiguredo::mp4::writer::FaststartWriter>(
            m_ofs,
            shiguredo::mp4::writer::FaststartWriterParameters{
                .mvhd_timescale = 1000,
                .duration = static_cast<float>(m_duration),
                .mdat_path_templete =
                    directory_for_faststart_intermediate_file.string() +
                    std::filesystem::path::preferred_separator + "mdatXXXXXX"});
  }

  initialize(m_config, m_faststart_writer);
}

void FaststartMP4Muxer::run() {
  m_faststart_writer->writeFtypBox();

  mux();

  if (m_vide_track) {
    m_faststart_writer->appendTrakAndUdtaBoxInfo(
        {m_vide_track.get(), m_soun_track.get()});
  } else {
    m_faststart_writer->appendTrakAndUdtaBoxInfo({m_soun_track.get()});
  }
  m_faststart_writer->writeMoovBox();
  m_faststart_writer->writeMdatHeader();
  m_faststart_writer->copyMdatData();
}

void FaststartMP4Muxer::cleanUp() {
  if (m_faststart_writer &&
      std::filesystem::exists(m_faststart_writer->getIntermediateFilePath())) {
    m_faststart_writer->deleteIntermediateFile();
  }
}

}  // namespace hisui::muxer
