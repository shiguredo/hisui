#pragma once

#include <cstdint>
#include <memory>
#include <mutex>
#include <optional>
#include <queue>

#include <boost/cstdint.hpp>
#include <boost/rational.hpp>

#include "frame.hpp"
#include "video/composer.hpp"
#include "video/encoder.hpp"
#include "video/sequencer.hpp"

namespace hisui::muxer {

struct VideoProducerParameters {
  const bool show_progress_bar = true;
  const bool is_finished = false;
};

class VideoProducer {
 public:
  explicit VideoProducer(const VideoProducerParameters&);
  virtual ~VideoProducer() = default;

  virtual void produce();
  void bufferPop();
  std::optional<hisui::Frame> bufferFront();
  bool isFinished();

  virtual std::uint32_t getWidth() const;
  virtual std::uint32_t getHeight() const;
  std::uint32_t getFourcc() const;

 protected:
  std::shared_ptr<hisui::video::Sequencer> m_sequencer;
  std::shared_ptr<hisui::video::Encoder> m_encoder;
  std::shared_ptr<hisui::video::Composer> m_composer;

  std::queue<hisui::Frame> m_buffer;

  bool m_show_progress_bar;

  bool m_is_finished = false;

  std::mutex m_mutex_buffer;

  double m_duration;
  boost::rational<std::uint64_t> m_frame_rate;
};

}  // namespace hisui::muxer
