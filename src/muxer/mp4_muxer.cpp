#include "muxer/mp4_muxer.hpp"

#include <cstdint>
#include <filesystem>
#include <iterator>
#include <memory>
#include <string>
#include <vector>

#include <boost/cstdint.hpp>
#include <boost/rational.hpp>

#include "config.hpp"
#include "constants.hpp"
#include "metadata.hpp"
#include "muxer/audio_producer.hpp"
#include "muxer/multi_channel_vpx_video_producer.hpp"
#include "muxer/no_video_producer.hpp"
#include "muxer/opus_audio_producer.hpp"
#include "muxer/video_producer.hpp"
#include "muxer/vpx_video_producer.hpp"
#include "report/reporter.hpp"
#include "shiguredo/mp4/track/opus.hpp"
#include "shiguredo/mp4/track/soun.hpp"
#include "shiguredo/mp4/track/vide.hpp"
#include "shiguredo/mp4/track/vpx.hpp"
#include "shiguredo/mp4/writer/writer.hpp"

#ifdef USE_FDK_AAC
#include "muxer/fdk_aac_audio_producer.hpp"
#include "shiguredo/mp4/track/aac.hpp"
#endif

namespace hisui::muxer {

MP4Muxer::MP4Muxer(const MP4MuxerParameters& params)
    : m_duration(params.duration),
      m_audio_archives(params.audio_archive_items),
      m_normal_archives(params.normal_archives),
      m_preferred_archives(params.preferred_archives) {}

MP4Muxer::MP4Muxer(const MP4MuxerParametersForLayout& params)
    : m_duration(params.duration),
      m_audio_archives(params.audio_archive_items) {
  m_video_producer = params.video_producer;
}

void MP4Muxer::initialize(
    const hisui::Config& config_orig,
    std::shared_ptr<shiguredo::mp4::writer::Writer> writer) {
  m_writer = writer;
  hisui::Config config = config_orig;
  if (config.out_video_bit_rate == 0) {
    config.out_video_bit_rate =
        static_cast<std::uint32_t>(std::size(m_normal_archives)) *
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
    m_video_chunk_interval = 30;
    m_audio_chunk_interval = 100;
  } else {
    m_audio_chunk_interval = 1000;
    m_video_chunk_interval = 1000;
  }

  m_ofs = std::ofstream(config.out_filename, std::ios_base::binary);
  if (config.audio_only) {
    m_video_producer = std::make_shared<NoVideoProducer>();
    m_timescale_ratio.assign(1, 1);
  } else {
    if (!m_video_producer) {
      if (!std::empty(m_preferred_archives)) {
        m_video_producer = std::make_shared<MultiChannelVPXVideoProducer>(
            config, MultiChannelVPXVideoProducerParameters{
                        .normal_archives = m_normal_archives,
                        .preferred_archives = m_preferred_archives,
                        .duration = m_duration,
                        .timescale = 16000,
                    });
      } else {
        m_video_producer = std::make_shared<VPXVideoProducer>(
            config, VPXVideoProducerParameters{.archives = m_normal_archives,
                                               .duration = m_duration,
                                               .timescale = 16000});
      }
    }
    m_vide_track = std::make_shared<shiguredo::mp4::track::VPXTrack>(
        shiguredo::mp4::track::VPXTrackParameters{
            .timescale = 16000,
            .duration = static_cast<float>(m_duration),
            .track_id = m_writer->getAndUpdateNextTrackID(),
            .width = m_video_producer->getWidth(),
            .height = m_video_producer->getHeight(),
            .writer = m_writer.get()});
  }

  if (config.out_audio_codec == config::OutAudioCodec::FDK_AAC) {
#ifdef USE_FDK_AAC
    m_audio_producer = std::make_shared<FDKAACAudioProducer>(
        config, FDKAACAudioProducerParameters{.archives = m_audio_archives,
                                              .duration = m_duration});
    m_soun_track = std::make_shared<shiguredo::mp4::track::AACTrack>(
        shiguredo::mp4::track::AACTrackParameters{
            .timescale = 48000,
            .duration = static_cast<float>(m_duration),
            .track_id = m_writer->getAndUpdateNextTrackID(),
            .buffer_size_db = 0,
            .max_bitrate = config.out_aac_bit_rate,
            .avg_bitrate = config.out_aac_bit_rate,
            .writer = m_writer.get(),
        });
#else
    throw std::logic_error("AAC: inconsistent setting");
#endif
  } else {
    auto audio_producer = std::make_shared<OpusAudioProducer>(
        config, m_audio_archives, m_duration, 48000);
    const auto skip = audio_producer->getSkip();
    m_audio_producer = audio_producer;
    m_soun_track = std::make_shared<shiguredo::mp4::track::OpusTrack>(
        shiguredo::mp4::track::OpusTrackParameters{
            .pre_skip = static_cast<std::uint64_t>(skip),
            .duration = static_cast<float>(m_duration),
            .track_id = m_writer->getAndUpdateNextTrackID(),
            .writer = m_writer.get()});
  }

  if (!config.audio_only) {
    m_timescale_ratio.assign(m_soun_track->getTimescale(),
                             m_vide_track->getTimescale());
  }

  if (hisui::report::Reporter::hasInstance()) {
    hisui::report::Reporter::getInstance().registerOutput({
        .container = "MP4",
        .mux_type = config.mp4_muxer == config::MP4Muxer::Faststart
                        ? "faststart"
                        : "simple",
        .video_codec =
            config.audio_only ? "none"
            : m_video_producer->getFourcc() == hisui::Constants::VP9_FOURCC
                ? "vp9"
                : "vp8",
        .audio_codec = config.out_audio_codec == config::OutAudioCodec::FDK_AAC
                           ? "aac"
                           : "opus",
        .duration = m_duration,
    });
  }
}

MP4Muxer::~MP4Muxer() {
  m_ofs.close();
}

void MP4Muxer::appendAudio(hisui::Frame frame) {
  if ((frame.timestamp * m_writer->getMvhdTimescale() /
       m_soun_track->getTimescale()) >=
          m_chunk_start + m_audio_chunk_interval ||
      std::size(m_audio_buffer) >= 2) {
    m_chunk_start += m_audio_chunk_interval;
    writeTrackData();
  }

  m_audio_buffer.push_back(frame);
  m_audio_producer->bufferPop();
}

void MP4Muxer::appendVideo(hisui::Frame frame) {
  if ((frame.timestamp * m_writer->getMvhdTimescale() /
       m_vide_track->getTimescale()) >=
          m_chunk_start + m_video_chunk_interval ||
      std::size(m_video_buffer) >= 1) {
    m_chunk_start += m_video_chunk_interval;
    writeTrackData();
  }

  m_video_buffer.push_back(frame);
  m_video_producer->bufferPop();
}

void MP4Muxer::writeTrackData() {
  for (const auto f : m_audio_buffer) {
    m_soun_track->addMdatData(f.timestamp, f.data, f.data_size, f.is_key);
    delete[] f.data;
  }
  m_soun_track->terminateCurrentChunk();
  m_audio_buffer.clear();
  if (!m_vide_track) {
    return;
  }
  for (const auto f : m_video_buffer) {
    m_vide_track->addMdatData(f.timestamp, f.data, f.data_size, f.is_key);
    delete[] f.data;
  }
  m_vide_track->terminateCurrentChunk();
  m_video_buffer.clear();
}

void MP4Muxer::muxFinalize() {
  writeTrackData();
}

}  // namespace hisui::muxer
