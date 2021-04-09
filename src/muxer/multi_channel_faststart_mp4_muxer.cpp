#include "muxer/multi_channel_faststart_mp4_muxer.hpp"

#include <bits/exception.h>
#include <spdlog/spdlog.h>

#include <filesystem>
#include <iosfwd>
#include <stdexcept>
#include <string>

#include "config.hpp"
#include "metadata.hpp"
#include "muxer/multi_channel_vpx_video_producer.hpp"
#include "muxer/opus_audio_producer.hpp"
#include "shiguredo/mp4/track/opus.hpp"
#include "shiguredo/mp4/track/soun.hpp"
#include "shiguredo/mp4/track/vide.hpp"
#include "shiguredo/mp4/track/vpx.hpp"
#include "shiguredo/mp4/writer/faststart_writer.hpp"

#ifdef USE_FDK_AAC
#include "muxer/fdk_aac_audio_producer.hpp"
#include "shiguredo/mp4/track/aac.hpp"
#endif

namespace shiguredo::mp4::track {

class Track;

}

namespace hisui::muxer {

MultiChannelFaststartMP4Muxer::MultiChannelFaststartMP4Muxer(
    const hisui::Config& t_config,
    const hisui::Metadata& t_metadata,
    const hisui::Metadata& t_multi_channel_metadata)
    : m_config(t_config),
      m_metadata(t_metadata),
      m_multi_channel_metadata(t_multi_channel_metadata) {}

void MultiChannelFaststartMP4Muxer::setUp() {
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
    if (metadata_path.is_relative()) {
      metadata_path = std::filesystem::absolute(metadata_path);
    }
    directory_for_faststart_intermediate_file = metadata_path.parent_path();
  }
  spdlog::debug("directory_for_faststart_intermediate_file: {}",
                directory_for_faststart_intermediate_file.string());

  const float duration = static_cast<float>(m_metadata.getMaxStopTimeOffset());
  m_faststart_writer = new shiguredo::mp4::writer::FaststartWriter(
      m_ofs, {.mvhd_timescale = 1000,
              .duration = duration,
              .mdat_path_templete =
                  directory_for_faststart_intermediate_file.string() +
                  std::filesystem::path::preferred_separator + "mdatXXXXXX"});
  initialize(m_config, m_metadata, m_multi_channel_metadata, m_faststart_writer,
             duration);
}

void MultiChannelFaststartMP4Muxer::initialize(
    const hisui::Config& config_orig,
    const hisui::Metadata& metadata,
    const hisui::Metadata& multi_channel_metadata,
    shiguredo::mp4::writer::Writer* writer,
    const float duration) {
  m_writer = writer;
  hisui::Config config = config_orig;
  if (config.out_video_bit_rate == 0) {
    config.out_video_bit_rate =
        static_cast<std::uint32_t>(std::size(metadata.getArchives())) *
        hisui::Constants::VIDEO_VPX_BIT_RATE_PER_FILE;
  } else {
    config.out_video_bit_rate = config.out_video_bit_rate;
  }

  if (config.out_filename == "") {
    std::filesystem::path metadata_path(config.in_metadata_filename);
    if (config.audio_only) {
      config.out_filename = metadata_path.replace_extension(".m4a");
    } else {
      config.out_filename = metadata_path.replace_extension(".mp4");
    }
  }

  if (config.out_audio_codec == config::OutAudioCodec::FDK_AAC) {
    m_chunk_interval = 960;  // 960ms
  } else {
    m_chunk_interval = 1000;  // 1000 ms
  }

  m_ofs = std::ofstream(config.out_filename, std::ios_base::binary);

  if (config.out_audio_codec == config::OutAudioCodec::FDK_AAC) {
#ifdef USE_FDK_AAC
    m_audio_producer = new FDKAACAudioProducer(config, metadata);
    m_soun_track = new shiguredo::mp4::track::AACTrack({
        .timescale = 48000,
        .duration = duration,
        .track_id = m_writer->getAndUpdateNextTrackID(),
        .max_bitrate = config.out_aac_bit_rate,
        .avg_bitrate = config.out_aac_bit_rate,
        .writer = m_writer,
    });
#else
    throw std::logic_error("AAC: inconsistent setting");
#endif
  } else {
    OpusAudioProducer* audio_producer =
        new OpusAudioProducer(config, metadata, 48000);
    const auto skip = audio_producer->getSkip();
    m_audio_producer = audio_producer;
    m_soun_track = new shiguredo::mp4::track::OpusTrack(
        {.pre_skip = static_cast<std::uint64_t>(skip),
         .duration = duration,
         .track_id = m_writer->getAndUpdateNextTrackID(),
         .writer = m_writer});
  }

  m_video_producer = new MultiChannelVPXVideoProducer(
      config, metadata, multi_channel_metadata, 16000);
  m_vide_track = new shiguredo::mp4::track::VPXTrack(
      {.timescale = 16000,
       .duration = duration,
       .track_id = m_writer->getAndUpdateNextTrackID(),
       .width = m_video_producer->getWidth(),
       .height = m_video_producer->getHeight(),
       .writer = m_writer});

  m_timescale_ratio.assign(m_soun_track->getTimescale(),
                           m_vide_track->getTimescale());
}

MultiChannelFaststartMP4Muxer::~MultiChannelFaststartMP4Muxer() {
  delete m_faststart_writer;
}

void MultiChannelFaststartMP4Muxer::run() {
  m_faststart_writer->writeFtypBox();

  mux();

  if (m_vide_track) {
    m_faststart_writer->appendTrakAndUdtaBoxInfo({m_soun_track, m_vide_track});
  } else {
    m_faststart_writer->appendTrakAndUdtaBoxInfo({m_soun_track});
  }
  m_faststart_writer->writeMoovBox();
  m_faststart_writer->writeMdatHeader();
  m_faststart_writer->copyMdatData();
}

void MultiChannelFaststartMP4Muxer::cleanUp() {
  if (std::filesystem::exists(m_faststart_writer->getIntermediateFilePath())) {
    m_faststart_writer->deleteIntermediateFile();
  }
}

}  // namespace hisui::muxer
