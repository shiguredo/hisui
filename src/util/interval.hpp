#pragma once

#include <cstdint>

namespace hisui::util {

class Interval {
 public:
  Interval(const std::uint64_t, const std::uint64_t);

  bool isIn(const std::uint64_t) const;
  std::uint64_t getSubstructLower(const std::uint64_t) const;

  void set(const std::uint64_t, const std::uint64_t);
  std::uint64_t getLower() const;
  std::uint64_t getUpper() const;

 private:
  std::uint64_t m_lower;
  std::uint64_t m_upper;
};

}  // namespace hisui::util
