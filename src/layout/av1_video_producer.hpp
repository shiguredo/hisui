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

struct AV1VideoProducerParameters {
  const std::vector<std::shared_ptr<Region>>& regions;
  const Resolution& resolution;
  const double duration;
  const std::uint64_t timescale = hisui::Constants::NANO_SECOND;
};

class AV1VideoProducer : public hisui::muxer::VideoProducer {
 public:
  AV1VideoProducer(const hisui::Config&, const AV1VideoProducerParameters&);
  void produce() override;
  std::uint32_t getWidth() const override;
  std::uint32_t getHeight() const override;
  const std::vector<std::uint8_t>& getExtraData() const override;

 private:
  Resolution m_resolution;
  std::shared_ptr<Composer> m_layout_composer;
};

}  // namespace hisui::layout
