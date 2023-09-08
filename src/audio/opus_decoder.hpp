#pragma once

#include <opus.h>

#include <cstddef>
#include <cstdint>
#include <utility>

#include "audio/decoder.hpp"

namespace hisui::audio {

class OpusDecoder : public Decoder {
 public:
  explicit OpusDecoder(const int t_channles);
  ~OpusDecoder();

  std::pair<const std::int16_t*, const std::size_t> decode(
      const unsigned char*,
      const std::size_t) override;

 private:
  ::OpusDecoder* m_decoder = nullptr;
  int m_channels;
  std::int16_t* m_opus_buffer = nullptr;
};

}  // namespace hisui::audio
