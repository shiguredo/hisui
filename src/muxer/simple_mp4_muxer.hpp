#pragma once

#include "config.hpp"
#include "metadata.hpp"
#include "muxer/mp4_muxer.hpp"

namespace shiguredo::mp4::writer {

class SimpleWriter;

}

namespace hisui::muxer {

class SimpleMP4Muxer : public MP4Muxer {
 public:
  SimpleMP4Muxer(const hisui::Config&, const hisui::MetadataSet&);
  ~SimpleMP4Muxer();

  void setUp() override;
  void run() override;
  void cleanUp() override;

 private:
  shiguredo::mp4::writer::SimpleWriter* m_simple_writer;

  hisui::Config m_config;
  hisui::MetadataSet m_metadata_set;
};

}  // namespace hisui::muxer
