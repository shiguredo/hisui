#pragma once

#include <opus.h>
#include <opus_types.h>

#include <cstdint>
#include <queue>
#include <vector>

#include "audio/encoder.hpp"
#include "constants.hpp"

namespace hisui {

struct Frame;

}

namespace hisui::audio {

struct BufferOpusEncoderParameters {
  const std::uint32_t bit_rate;
  const std::uint64_t timescale = hisui::Constants::NANO_SECOND;
};

class BufferOpusEncoder : public Encoder {
 public:
  explicit BufferOpusEncoder(std::queue<hisui::Frame>*,
                             const BufferOpusEncoderParameters&);
  ~BufferOpusEncoder();
  void addSample(const std::int16_t, const std::int16_t) override;
  void flush() override;

  ::opus_int32 getSkip() const;

 private:
  std::queue<hisui::Frame>* m_buffer;
  ::OpusEncoder* m_encoder;
  std::vector<opus_int16> m_pcm_buffer;
  std::uint8_t m_opus_buffer[hisui::Constants::OPUS_MAX_PACKET_SIZE];
  std::uint64_t m_timestamp = 0;
  const std::uint64_t m_timescale;
  const std::uint64_t m_timestamp_step;
  ::opus_int32 m_skip;

  void encodeAndWrite();
};

}  // namespace hisui::audio
