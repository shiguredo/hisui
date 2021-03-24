#pragma once

#include <cstdint>
#include <mutex>
#include <optional>
#include <queue>

#include "frame.hpp"

namespace hisui::audio {

class Encoder;
class Sequencer;

}  // namespace hisui::audio

namespace hisui::muxer {

struct AudioProducerParameters {
  const bool show_progress_bar = true;
};

class AudioProducer {
 public:
  explicit AudioProducer(const AudioProducerParameters&);
  virtual ~AudioProducer();
  void produce();
  void bufferPop();
  std::optional<hisui::Frame> bufferFront();
  bool isFinished();

 protected:
  std::queue<hisui::Frame> m_buffer;
  hisui::audio::Sequencer* m_sequencer;
  std::int16_t (*m_mix_sample)(const std::int16_t, const std::int16_t);
  hisui::audio::Encoder* m_encoder;
  double m_max_stop_time_offset;

  std::mutex m_mutex_buffer;

  bool m_show_progress_bar;
  bool m_is_finished = false;
};

}  // namespace hisui::muxer
