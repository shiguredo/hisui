#pragma once

#include <cstdint>
#include <memory>
#include <mutex>
#include <optional>
#include <queue>
#include <vector>

#include "archive_item.hpp"
#include "audio/encoder.hpp"
#include "audio/sequencer.hpp"
#include "config.hpp"
#include "frame.hpp"

namespace hisui::muxer {

struct AudioProducerParameters {
  const std::vector<hisui::ArchiveItem>& archives;
  const hisui::config::AudioMixer mixer;
  const double duration;
  const bool show_progress_bar = true;
};

class AudioProducer {
 public:
  explicit AudioProducer(const AudioProducerParameters&);
  virtual ~AudioProducer() = default;
  void produce();
  void bufferPop();
  std::optional<hisui::Frame> bufferFront();
  bool isFinished();

 protected:
  std::shared_ptr<hisui::audio::Encoder> m_encoder;
  std::queue<hisui::Frame> m_buffer;

 private:
  std::unique_ptr<hisui::audio::Sequencer> m_sequencer;
  std::int16_t (*m_mix_sample)(const std::int16_t, const std::int16_t);
  double m_duration;

  std::mutex m_mutex_buffer;

  bool m_show_progress_bar;
  bool m_is_finished = false;
};

}  // namespace hisui::muxer
