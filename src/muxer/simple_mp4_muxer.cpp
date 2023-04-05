#include "muxer/simple_mp4_muxer.hpp"

#include <iosfwd>
#include <vector>

#include "metadata.hpp"
#include "shiguredo/mp4/track/soun.hpp"
#include "shiguredo/mp4/track/vide.hpp"
#include "shiguredo/mp4/writer/simple_writer.hpp"

namespace shiguredo::mp4::track {

class Track;

}

namespace hisui::muxer {

SimpleMP4Muxer::SimpleMP4Muxer(const hisui::Config& t_config,
                               const MP4MuxerParameters& params)
    : MP4Muxer(params), m_config(t_config) {}

SimpleMP4Muxer::SimpleMP4Muxer(const hisui::Config& t_config,
                               const MP4MuxerParametersForLayout& params)
    : MP4Muxer(params), m_config(t_config) {}

void SimpleMP4Muxer::setUp() {
  m_simple_writer = std::make_shared<shiguredo::mp4::writer::SimpleWriter>(
      m_ofs,
      shiguredo::mp4::writer::SimpleWriterParameters{
          .mvhd_timescale = 1000, .duration = static_cast<float>(m_duration)});
  initialize(m_config, m_simple_writer);
}

void SimpleMP4Muxer::run() {
  m_simple_writer->writeFtypBox();

  mux();

  if (m_vide_track) {
    m_simple_writer->appendTrakAndUdtaBoxInfo(
        {m_soun_track.get(), m_vide_track.get()});
  } else {
    m_simple_writer->appendTrakAndUdtaBoxInfo({m_soun_track.get()});
  }
  m_simple_writer->writeFreeBoxAndMdatHeader();
  m_simple_writer->writeMoovBox();
}

void SimpleMP4Muxer::cleanUp() {}

}  // namespace hisui::muxer
