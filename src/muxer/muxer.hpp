#pragma once

#include <cstdint>
#include <memory>

#include <boost/cstdint.hpp>
#include <boost/rational.hpp>

namespace hisui {

struct Frame;

}

namespace hisui::muxer {

class AudioProducer;
class VideoProducer;

class Muxer {
 public:
  virtual ~Muxer() = default;
  virtual void setUp() = 0;
  virtual void run() = 0;
  virtual void cleanUp() = 0;

 protected:
  void mux();

  std::shared_ptr<VideoProducer> m_video_producer;
  std::shared_ptr<AudioProducer> m_audio_producer;
  boost::rational<std::uint64_t> m_timescale_ratio = 1;

 private:
  virtual void muxFinalize() = 0;
  virtual void appendAudio(hisui::Frame) = 0;
  virtual void appendVideo(hisui::Frame) = 0;
};

}  // namespace hisui::muxer
