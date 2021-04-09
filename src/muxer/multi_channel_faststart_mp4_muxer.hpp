#pragma once

#include "config.hpp"
#include "metadata.hpp"
#include "muxer/mp4_muxer.hpp"

namespace shiguredo::mp4::writer {

class FaststartWriter;

}

namespace hisui::muxer {

class MultiChannelFaststartMP4Muxer : public MP4Muxer {
 public:
  MultiChannelFaststartMP4Muxer(const hisui::Config&,
                                const hisui::Metadata&,
                                const hisui::Metadata&);
  ~MultiChannelFaststartMP4Muxer();

  void setUp() override;
  void run() override;
  void cleanUp() override;
  void initialize(const hisui::Config&,
                  const hisui::Metadata&,
                  const hisui::Metadata&,
                  shiguredo::mp4::writer::Writer*,
                  const float);

 private:
  shiguredo::mp4::writer::FaststartWriter* m_faststart_writer;

  hisui::Config m_config;
  hisui::Metadata m_metadata;
  hisui::Metadata m_multi_channel_metadata;
};

}  // namespace hisui::muxer
