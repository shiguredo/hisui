#pragma once

#include <cstdio>

#include "config.hpp"
#include "metadata.hpp"
#include "muxer/multi_channel_vpx_video_producer.hpp"
#include "muxer/muxer.hpp"

namespace hisui {

struct Frame;

}

namespace hisui::webm::output {

class Context;

}  // namespace hisui::webm::output

namespace hisui::muxer {

class MultiChannelVPXVideoProducer;

class MultiChannelAsyncWebMMuxer : public Muxer {
 public:
  MultiChannelAsyncWebMMuxer(const hisui::Config&,
                             const hisui::Metadata&,
                             const hisui::Metadata&);
  ~MultiChannelAsyncWebMMuxer();

  void setUp() override;
  void run() override;
  void cleanUp() override;

 private:
  void muxFinalize() override;
  void appendAudio(hisui::Frame) override;
  void appendVideo(hisui::Frame) override;

  hisui::webm::output::Context* m_context;
  std::FILE* m_file;

  hisui::Config m_config;
  hisui::Metadata m_metadata;
  hisui::Metadata m_multi_channel_metadata;
};

}  // namespace hisui::muxer
