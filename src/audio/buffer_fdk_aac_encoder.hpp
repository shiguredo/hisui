#pragma once

#include <fdk-aac/aacenc_lib.h>

#include <cstdint>
#include <queue>
#include <vector>

#include "audio/encoder.hpp"

namespace hisui {

struct Frame;

}

namespace hisui::audio {

struct BufferFDKAACEncoderParameters {
  const std::uint32_t bit_rate;
};

class BufferFDKAACEncoder : public Encoder {
 public:
  BufferFDKAACEncoder(std::queue<hisui::Frame>*,
                      const BufferFDKAACEncoderParameters&);
  ~BufferFDKAACEncoder();
  void addSample(const std::int16_t, const std::int16_t) override;
  void flush() override;

 private:
  std::queue<hisui::Frame>* m_buffer;
  ::HANDLE_AACENCODER m_handle;
  std::vector<std::int16_t> m_pcm_buffer;
  std::uint64_t m_max_sample_size;
  std::uint8_t* m_aac_buffer = nullptr;
  std::uint64_t m_timestamp = 0;

  void encodeAndWrite();
};

}  // namespace hisui::audio
