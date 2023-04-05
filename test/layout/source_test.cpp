#include <boost/test/unit_test.hpp>

#include "layout/interval.hpp"
#include "layout/source.hpp"

BOOST_AUTO_TEST_SUITE(source)

BOOST_AUTO_TEST_CASE(substruct_trim_intervals) {
  {
    auto expected = hisui::layout::Interval{.start_time = 0, .end_time = 10};
    BOOST_REQUIRE_EQUAL(expected,
                        hisui::layout::substruct_trim_intervals(
                            {.interval = {.start_time = 0, .end_time = 10},
                             .trim_intervals = {}}));
  }
  {
    auto expected = hisui::layout::Interval{.start_time = 0, .end_time = 10};
    BOOST_REQUIRE_EQUAL(expected,
                        hisui::layout::substruct_trim_intervals(
                            {.interval = {.start_time = 0, .end_time = 10},
                             .trim_intervals = {{100, 200}}}));
  }
  {
    auto expected = hisui::layout::Interval{.start_time = 5, .end_time = 10};
    BOOST_REQUIRE_EQUAL(expected,
                        hisui::layout::substruct_trim_intervals(
                            {.interval = {.start_time = 10, .end_time = 15},
                             .trim_intervals = {{0, 5}}}));
  }
  {
    auto expected = hisui::layout::Interval{.start_time = 5, .end_time = 10};
    BOOST_REQUIRE_EQUAL(expected,
                        hisui::layout::substruct_trim_intervals(
                            {.interval = {.start_time = 10, .end_time = 15},
                             .trim_intervals = {{0, 5}, {15, 20}}}));
  }
  {
    auto expected = hisui::layout::Interval{.start_time = 5, .end_time = 10};
    BOOST_REQUIRE_EQUAL(expected,
                        hisui::layout::substruct_trim_intervals(
                            {.interval = {.start_time = 10, .end_time = 15},
                             .trim_intervals = {{0, 2}, {5, 8}}}));
  }
  {
    auto expected = hisui::layout::Interval{.start_time = 5, .end_time = 10};
    BOOST_REQUIRE_EQUAL(expected,
                        hisui::layout::substruct_trim_intervals(
                            {.interval = {.start_time = 10, .end_time = 15},
                             .trim_intervals = {{0, 2}, {5, 8}, {100, 200}}}));
  }
}

BOOST_AUTO_TEST_SUITE_END()
