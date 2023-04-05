#include "util/interval.hpp"

namespace hisui::util {

Interval::Interval(const std::uint64_t t_lower, const std::uint64_t t_upper)
    : m_lower(t_lower), m_upper(t_upper) {}

bool Interval::isIn(const std::uint64_t t) const {
  return m_lower <= t && t < m_upper;
}
std::uint64_t Interval::getSubstructLower(const std::uint64_t t) const {
  return t - m_lower;
}

void Interval::set(const std::uint64_t l, const std::uint64_t u) {
  m_lower = l;
  m_upper = u;
}

std::uint64_t Interval::getLower() const {
  return m_lower;
}

std::uint64_t Interval::getUpper() const {
  return m_upper;
}

}  // namespace hisui::util
