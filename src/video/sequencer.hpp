#pragma once

#include <cstddef>
#include <cstdint>
#include <memory>
#include <string>
#include <utility>
#include <vector>

#include "util/interval.hpp"

namespace hisui {

class Archive;

}

namespace hisui::video {

class Source;
class YUVImage;

using SourceAndInterval =
    std::pair<std::unique_ptr<Source>, hisui::util::Interval>;

struct SequencerGetYUVsResult {
  const bool is_preferred_stream = false;
};

class Sequencer {
 public:
  virtual ~Sequencer() = default;

  virtual SequencerGetYUVsResult getYUVs(std::vector<const YUVImage*>*,
                                         const std::uint64_t) = 0;

  std::uint32_t getMaxWidth() const;
  std::uint32_t getMaxHeight() const;
  std::size_t getSize() const;

 protected:
  std::vector<
      std::pair<std::string, std::shared_ptr<std::vector<SourceAndInterval>>>>
      m_sequence;
  std::uint32_t m_max_width;
  std::uint32_t m_max_height;
  std::size_t m_size;
};

struct MakeSequenceResult {
  std::vector<
      std::pair<std::string, std::shared_ptr<std::vector<SourceAndInterval>>>>
      sequence{};
  std::uint32_t max_width = 0;
  std::uint32_t max_height = 0;
};

MakeSequenceResult make_sequence(const std::vector<hisui::Archive>&);

}  // namespace hisui::video
