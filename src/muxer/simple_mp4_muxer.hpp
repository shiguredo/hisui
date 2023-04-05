#pragma once

#include <memory>

#include "config.hpp"
#include "metadata.hpp"
#include "muxer/mp4_muxer.hpp"

namespace shiguredo::mp4::writer {

class SimpleWriter;

}

namespace hisui::muxer {

class SimpleMP4Muxer : public MP4Muxer {
 public:
  SimpleMP4Muxer(const hisui::Config&, const MP4MuxerParameters&);
  SimpleMP4Muxer(const hisui::Config&, const MP4MuxerParametersForLayout&);

  void setUp() override;
  void run() override;
  void cleanUp() override;

 private:
  std::shared_ptr<shiguredo::mp4::writer::SimpleWriter> m_simple_writer;

  hisui::Config m_config;
};

}  // namespace hisui::muxer
