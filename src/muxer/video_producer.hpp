#pragma once

#include <cstdint>
#include <mutex>
#include <optional>
#include <queue>

#include <boost/cstdint.hpp>
#include <boost/rational.hpp>

namespace hisui {

struct Frame;

}

namespace hisui::video {

class Encoder;
class Sequencer;
class Composer;

}  // namespace hisui::video

namespace hisui::muxer {

struct VideoProducerParameters {
  const bool show_progress_bar = true;
  const bool is_finished = false;
};

class VideoProducer {
 public:
  explicit VideoProducer(const VideoProducerParameters&);
  virtual ~VideoProducer();
  virtual void produce();
  void bufferPop();
  std::optional<hisui::Frame> bufferFront();
  bool isFinished();

  std::uint32_t getWidth() const;
  std::uint32_t getHeight() const;
  std::uint32_t getFourcc() const;

 protected:
  hisui::video::Sequencer* m_sequencer = nullptr;
  hisui::video::Encoder* m_encoder = nullptr;
  hisui::video::Composer* m_composer = nullptr;

  std::queue<hisui::Frame> m_buffer;

  bool m_show_progress_bar;

  bool m_is_finished = false;

  std::mutex m_mutex_buffer;

  double m_max_stop_time_offset;
  boost::rational<std::uint64_t> m_frame_rate;
};

}  // namespace hisui::muxer
