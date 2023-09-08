#pragma once

#include <cstdint>
#include <memory>

#include "config.hpp"
#include "video/decoder.hpp"

namespace hisui::webm::input {

class VideoContext;

}

namespace hisui::video {

class DecoderFactory {
 public:
  DecoderFactory(const DecoderFactory&) = delete;
  DecoderFactory& operator=(const DecoderFactory&) = delete;
  DecoderFactory(DecoderFactory&&) = delete;
  DecoderFactory& operator=(DecoderFactory&&) = delete;

  static std::shared_ptr<hisui::video::Decoder> create(
      std::shared_ptr<hisui::webm::input::VideoContext>);
  static void setup(const hisui::Config&);

 private:
  explicit DecoderFactory(const hisui::Config&);

  inline static std::unique_ptr<DecoderFactory> m_instance = nullptr;
  hisui::Config m_config;
};

}  // namespace hisui::video
