#include "muxer/simple_mp4_muxer.hpp"

#include <iosfwd>

#include "metadata.hpp"
#include "shiguredo/mp4/track/soun.hpp"
#include "shiguredo/mp4/track/vide.hpp"
#include "shiguredo/mp4/writer/simple_writer.hpp"

namespace shiguredo::mp4::track {

class Track;

}

namespace hisui::muxer {

SimpleMP4Muxer::SimpleMP4Muxer(const hisui::Config& t_config,
                               const hisui::Metadata& t_metadata)
    : m_config(t_config), m_metadata(t_metadata) {}

void SimpleMP4Muxer::setUp() {
  const float duration = static_cast<float>(m_metadata.getMaxStopTimeOffset());
  m_simple_writer = new shiguredo::mp4::writer::SimpleWriter(
      m_ofs, {.mvhd_timescale = 1000, .duration = duration});
  initialize(m_config, m_metadata, m_simple_writer, duration);
}

SimpleMP4Muxer::~SimpleMP4Muxer() {
  delete m_simple_writer;
}

void SimpleMP4Muxer::run() {
  m_simple_writer->writeFtypBox();

  mux();

  if (m_vide_track) {
    m_simple_writer->appendTrakAndUdtaBoxInfo({m_soun_track, m_vide_track});
  } else {
    m_simple_writer->appendTrakAndUdtaBoxInfo({m_soun_track});
  }
  m_simple_writer->writeFreeBoxAndMdatHeader();
  m_simple_writer->writeMoovBox();
}

void SimpleMP4Muxer::cleanUp() {}

}  // namespace hisui::muxer
