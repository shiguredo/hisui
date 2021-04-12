#pragma once

#include <vector>

#include "config.hpp"
#include "metadata.hpp"
#include "muxer/mp4_muxer.hpp"

namespace shiguredo::mp4::writer {

class FaststartWriter;

}

namespace hisui::muxer {

class FaststartMP4Muxer : public MP4Muxer {
 public:
  FaststartMP4Muxer(const hisui::Config&, const std::vector<hisui::Metadata>&);
  ~FaststartMP4Muxer();

  void setUp() override;
  void run() override;
  void cleanUp() override;

 private:
  shiguredo::mp4::writer::FaststartWriter* m_faststart_writer;

  hisui::Config m_config;
  std::vector<hisui::Metadata> m_metadata_list;
};

}  // namespace hisui::muxer
