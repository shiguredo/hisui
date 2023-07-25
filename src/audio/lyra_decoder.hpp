#pragma once

#include <lyra.h>

#include <cstddef>
#include <cstdint>
#include <string>
#include <utility>

#include "audio/decoder.hpp"

namespace hisui::audio {

class LyraDecoder : public Decoder {
 public:
  explicit LyraDecoder(const int t_channles, const std::string& model_path);
  ~LyraDecoder();

  std::pair<const std::int16_t*, const std::size_t> decode(
      const unsigned char*,
      const std::size_t) override;

 private:
  lyra_decoder* m_decoder;
  int m_channels;
  std::int16_t* m_lyra_buffer = nullptr;
};

}  // namespace hisui::audio
