#pragma once

#include <cstdint>
#include <memory>
#include <vector>

#include "constants.hpp"
#include "layout/cell_util.hpp"
#include "layout/composer.hpp"
#include "layout/metadata.hpp"
#include "muxer/video_producer.hpp"

namespace hisui {

class Config;

}  // namespace hisui

namespace hisui::layout {

struct VPXVideoProducerParameters {
  const std::vector<std::shared_ptr<Region>>& regions;
  const Resolution& resolution;
  const double duration;
  const std::uint64_t timescale = hisui::Constants::NANO_SECOND;
};

class VPXVideoProducer : public hisui::muxer::VideoProducer {
 public:
  VPXVideoProducer(const hisui::Config&, const VPXVideoProducerParameters&);
  void produce() override;
  std::uint32_t getWidth() const override;
  std::uint32_t getHeight() const override;

 private:
  Resolution m_resolution;
  std::shared_ptr<Composer> m_layout_composer;
};

}  // namespace hisui::layout
