#include "muxer/mp4_muxer.hpp"

#include <cstdint>
#include <filesystem>
#include <iterator>
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

void MP4Muxer::initialize(const hisui::Config& config_orig,
                          const hisui::MetadataSet& metadata_set,
                          shiguredo::mp4::writer::Writer* writer,
                          const float duration) {
  m_writer = writer;
  hisui::Config config = config_orig;
  if (config.out_video_bit_rate == 0) {
    config.out_video_bit_rate = static_cast<std::uint32_t>(std::size(
                                    metadata_set.getNormalArchives())) *
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
    m_audio_producer = new FDKAACAudioProducer(config, metadata_set);
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
        new OpusAudioProducer(config, metadata_set, 48000);
    const auto skip = audio_producer->getSkip();
    m_audio_producer = audio_producer;
    m_soun_track = new shiguredo::mp4::track::OpusTrack(
        {.pre_skip = static_cast<std::uint64_t>(skip),
         .duration = duration,
         .track_id = m_writer->getAndUpdateNextTrackID(),
         .writer = m_writer});
  }

  if (config.audio_only) {
    m_video_producer = new NoVideoProducer();
    m_timescale_ratio.assign(1, 1);
  } else {
    if (metadata_set.hasPreferred()) {
      m_video_producer =
          new MultiChannelVPXVideoProducer(config, metadata_set, 16000);
    } else {
      m_video_producer =
          new VPXVideoProducer(config, metadata_set.getNormal(), 16000);
    }
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
}

MP4Muxer::~MP4Muxer() {
  m_ofs.close();

  if (m_vide_track) {
    delete m_vide_track;
  }
  delete m_soun_track;
  delete m_video_producer;
  delete m_audio_producer;
}

void MP4Muxer::appendAudio(hisui::Frame frame) {
  if ((frame.timestamp * m_writer->getMvhdTimescale() /
       m_soun_track->getTimescale()) >= m_chunk_start + m_chunk_interval) {
    m_chunk_start += m_chunk_interval;
    writeTrackData();
  }

  m_audio_buffer.push_back(frame);
  m_audio_producer->bufferPop();
}

void MP4Muxer::appendVideo(hisui::Frame frame) {
  if ((frame.timestamp * m_writer->getMvhdTimescale() /
       m_vide_track->getTimescale()) >= m_chunk_start + m_chunk_interval) {
    m_chunk_start += m_chunk_interval;
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
