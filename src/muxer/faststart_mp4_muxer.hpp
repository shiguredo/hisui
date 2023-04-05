#pragma once

#include <memory>

#include "config.hpp"
#include "metadata.hpp"
#include "muxer/mp4_muxer.hpp"

namespace shiguredo::mp4::writer {

class FaststartWriter;

}

namespace hisui::muxer {

class FaststartMP4Muxer : public MP4Muxer {
 public:
  FaststartMP4Muxer(const hisui::Config&, const MP4MuxerParameters&);
  FaststartMP4Muxer(const hisui::Config&, const MP4MuxerParametersForLayout&);

  void setUp() override;
  void run() override;
  void cleanUp() override;

 private:
  std::shared_ptr<shiguredo::mp4::writer::FaststartWriter> m_faststart_writer;

  hisui::Config m_config;
};

}  // namespace hisui::muxer
