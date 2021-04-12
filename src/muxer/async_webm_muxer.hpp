#pragma once

#include <cstdio>
#include <vector>

#include "config.hpp"
#include "metadata.hpp"
#include "muxer/muxer.hpp"

namespace hisui {

struct Frame;

}

namespace hisui::webm::output {

class Context;

}  // namespace hisui::webm::output

namespace hisui::muxer {

class AsyncWebMMuxer : public Muxer {
 public:
  AsyncWebMMuxer(const hisui::Config&, const std::vector<hisui::Metadata>&);
  ~AsyncWebMMuxer();

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
  std::vector<hisui::Metadata> m_metadata_list;
};

}  // namespace hisui::muxer
