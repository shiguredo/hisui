#include <limits>
#include <vector>

#include <boost/test/unit_test.hpp>

#include "layout/interval.hpp"
#include "layout/overlap.hpp"
#include "layout/reuse.hpp"
#include "layout/source.hpp"

BOOST_AUTO_TEST_SUITE(overlap)

BOOST_AUTO_TEST_CASE(overlap_intervals) {
  {
    auto expected = hisui::layout::OverlapIntervalsResult{
        .max_number_of_overlap = 1,
        .min_start_time = 0,
        .max_end_time = 1,
        .trim_intervals = {{1, std::numeric_limits<double>::max()}},
    };

    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::overlap_intervals({
            .intervals =
                {
                    hisui::layout::Interval({.start_time = 0, .end_time = 1}),
                },
            .reuse = hisui::layout::Reuse::None,
        }));
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::overlap_intervals({
            .intervals =
                {
                    hisui::layout::Interval({.start_time = 0, .end_time = 1}),
                },
            .reuse = hisui::layout::Reuse::ShowOldest,
        }));

    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::overlap_intervals({
            .intervals =
                {
                    hisui::layout::Interval({.start_time = 0, .end_time = 1}),
                },
            .reuse = hisui::layout::Reuse::ShowNewest,
        }));
  }

  {
    auto expected = hisui::layout::OverlapIntervalsResult{
        .max_number_of_overlap = 2,
        .min_start_time = 0,
        .max_end_time = 2,
        .trim_intervals = {{2, std::numeric_limits<double>::max()}},
    };
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::overlap_intervals({
            .intervals =
                {
                    hisui::layout::Interval({.start_time = 0, .end_time = 2}),
                    hisui::layout::Interval({.start_time = 1, .end_time = 2}),
                },
            .reuse = hisui::layout::Reuse::None,
        }));

    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::overlap_intervals({
            .intervals =
                {
                    hisui::layout::Interval({.start_time = 0, .end_time = 2}),
                    hisui::layout::Interval({.start_time = 1, .end_time = 2}),
                },
            .reuse = hisui::layout::Reuse::ShowOldest,
        }));

    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::overlap_intervals({
            .intervals =
                {
                    hisui::layout::Interval({.start_time = 0, .end_time = 2}),
                    hisui::layout::Interval({.start_time = 1, .end_time = 2}),
                },
            .reuse = hisui::layout::Reuse::ShowNewest,
        }));
  }

  {
    auto expected = hisui::layout::OverlapIntervalsResult{
        .max_number_of_overlap = 2,
        .min_start_time = 0,
        .max_end_time = 3,
        .trim_intervals = {{3, std::numeric_limits<double>::max()}},
    };

    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::overlap_intervals({
            .intervals =
                {
                    hisui::layout::Interval({.start_time = 0, .end_time = 2}),
                    hisui::layout::Interval({.start_time = 2, .end_time = 3}),
                },
            .reuse = hisui::layout::Reuse::None,
        }));
  }
  {
    auto expected = hisui::layout::OverlapIntervalsResult{
        .max_number_of_overlap = 1,
        .max_end_time = 3,
        .trim_intervals = {{3, std::numeric_limits<double>::max()}},
    };

    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::overlap_intervals({
            .intervals =
                {
                    hisui::layout::Interval({.start_time = 0, .end_time = 2}),
                    hisui::layout::Interval({.start_time = 2, .end_time = 3}),
                },
            .reuse = hisui::layout::Reuse::ShowOldest,
        }));
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::overlap_intervals({
            .intervals =
                {
                    hisui::layout::Interval({.start_time = 0, .end_time = 2}),
                    hisui::layout::Interval({.start_time = 2, .end_time = 3}),
                },
            .reuse = hisui::layout::Reuse::ShowNewest,
        }));
  }

  {
    auto expected = hisui::layout::OverlapIntervalsResult{
        .max_number_of_overlap = 2,
        .min_start_time = 0,
        .max_end_time = 6,
        .trim_intervals = {{6, std::numeric_limits<double>::max()}},
    };

    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::overlap_intervals({
            .intervals =
                {
                    hisui::layout::Interval({.start_time = 0, .end_time = 2}),
                    hisui::layout::Interval({.start_time = 2, .end_time = 4}),
                    hisui::layout::Interval({.start_time = 3, .end_time = 6}),
                },
            .reuse = hisui::layout::Reuse::ShowOldest,
        }));
  }

  {
    auto expected = hisui::layout::OverlapIntervalsResult{
        .max_number_of_overlap = 3,
        .min_start_time = 0,
        .max_end_time = 8,
        .trim_intervals = {{8, std::numeric_limits<double>::max()}},
    };
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::overlap_intervals({
            .intervals =
                {
                    hisui::layout::Interval({.start_time = 0, .end_time = 8}),
                    hisui::layout::Interval({.start_time = 2, .end_time = 5}),
                    hisui::layout::Interval({.start_time = 5, .end_time = 6}),
                    hisui::layout::Interval({.start_time = 3, .end_time = 7}),
                },
            .reuse = hisui::layout::Reuse::ShowOldest,
        }));
  }

  {
    auto expected = hisui::layout::OverlapIntervalsResult{
        .max_number_of_overlap = 1,
        .min_start_time = 1,
        .max_end_time = 7,
        .trim_intervals = {{0, 1},
                           {4, 5},
                           {7, std::numeric_limits<double>::max()}},
    };
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::overlap_intervals({
            .intervals =
                {
                    hisui::layout::Interval({.start_time = 1, .end_time = 2}),
                    hisui::layout::Interval({.start_time = 2, .end_time = 4}),
                    hisui::layout::Interval({.start_time = 5, .end_time = 6}),
                    hisui::layout::Interval({.start_time = 6, .end_time = 7}),
                },
            .reuse = hisui::layout::Reuse::ShowOldest,
        }));
  }

  {
    auto expected = hisui::layout::OverlapIntervalsResult{
        .max_number_of_overlap = 2,
        .min_start_time = 1,
        .max_end_time = 7,
        .trim_intervals = {{0, 1},
                           {2, 3},
                           {4, 5},
                           {7, std::numeric_limits<double>::max()}},
    };
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::overlap_intervals({
            .intervals =
                {
                    hisui::layout::Interval({.start_time = 1, .end_time = 2}),
                    hisui::layout::Interval({.start_time = 3, .end_time = 4}),
                    hisui::layout::Interval({.start_time = 5, .end_time = 7}),
                    hisui::layout::Interval({.start_time = 6, .end_time = 7}),
                },
            .reuse = hisui::layout::Reuse::ShowOldest,
        }));
  }
}

BOOST_AUTO_TEST_CASE(overlap_trim_intervals) {
  {
    auto expected = hisui::layout::TrimIntervals{
        .trim_intervals = {{0, std::numeric_limits<double>::max()}},
    };
    BOOST_REQUIRE_EQUAL(expected,
                        hisui::layout::overlap_trim_intervals(
                            {.list_of_trim_intervals = {
                                 {{0, std::numeric_limits<double>::max()}}}}));
  }
  {
    auto expected = hisui::layout::TrimIntervals{
        .trim_intervals = {{0, 100},
                           {200, 300},
                           {400, std::numeric_limits<double>::max()}},
    };
    BOOST_REQUIRE_EQUAL(
        expected, hisui::layout::overlap_trim_intervals(
                      {.list_of_trim_intervals = {
                           {{0, 100},
                            {200, 300},
                            {400, std::numeric_limits<double>::max()}}}}));
  }

  {
    auto expected = hisui::layout::TrimIntervals{
        .trim_intervals = {{0, 100},
                           {200, 300},
                           {500, std::numeric_limits<double>::max()}},
    };
    BOOST_REQUIRE_EQUAL(expected,
                        hisui::layout::overlap_trim_intervals(
                            {.list_of_trim_intervals = {
                                 {{0, 100},
                                  {200, 300},
                                  {500, std::numeric_limits<double>::max()}},
                                 {{0, 200},
                                  {200, 400},
                                  {450, std::numeric_limits<double>::max()}},
                             }}));
  }

  {
    auto expected = hisui::layout::TrimIntervals{
        .trim_intervals = {{0, 100}, {500, std::numeric_limits<double>::max()}},
    };
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::overlap_trim_intervals(
            {.list_of_trim_intervals = {
                 {{0, 100},
                  {200, 300},
                  {500, std::numeric_limits<double>::max()}},
                 {{0, 200}, {400, std::numeric_limits<double>::max()}},
                 {{0, 400}, {450, std::numeric_limits<double>::max()}},
             }}));
  }

  {
    auto expected = hisui::layout::TrimIntervals{
        .trim_intervals = {{500, std::numeric_limits<double>::max()}},
    };
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::overlap_trim_intervals(
            {.list_of_trim_intervals = {
                 {{0, 100},
                  {200, 300},
                  {500, std::numeric_limits<double>::max()}},
                 {{0, 200}, {500, std::numeric_limits<double>::max()}},
                 {{500, std::numeric_limits<double>::max()}},
                 {{0, 400}, {500, std::numeric_limits<double>::max()}},
             }}));
  }

  {
    auto expected = hisui::layout::TrimIntervals{
        .trim_intervals = {{0, 100},
                           {250, 300},
                           {500, std::numeric_limits<double>::max()}},
    };
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::overlap_trim_intervals(
            {.list_of_trim_intervals = {
                 {{0, 100},
                  {200, 300},
                  {500, std::numeric_limits<double>::max()}},
                 {{0, 200},
                  {250, 300},
                  {500, std::numeric_limits<double>::max()}},
                 {{0, 400}, {500, std::numeric_limits<double>::max()}},
             }}));
  }

  {
    auto expected = hisui::layout::TrimIntervals{
        .trim_intervals = {{200, 300},
                           {500, std::numeric_limits<double>::max()}},
    };
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::overlap_trim_intervals(
            {.list_of_trim_intervals = {
                 {{0, 100},
                  {200, 350},
                  {500, std::numeric_limits<double>::max()}},
                 {{200, 300}, {500, std::numeric_limits<double>::max()}},
             }}));
  }

  {
    auto expected = hisui::layout::TrimIntervals{
        .trim_intervals = {{200, 300},
                           {500, std::numeric_limits<double>::max()}},
    };
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::overlap_trim_intervals(
            {.list_of_trim_intervals = {
                 {{200, 300}, {500, std::numeric_limits<double>::max()}},
                 {{0, 100},
                  {200, 350},
                  {500, std::numeric_limits<double>::max()}},
             }}));
  }

  {
    auto expected = hisui::layout::TrimIntervals{
        .trim_intervals = {{200, 300},
                           {325, 350},
                           {500, std::numeric_limits<double>::max()}},
    };
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::overlap_trim_intervals(
            {.list_of_trim_intervals = {
                 {{200, 300}, {325, std::numeric_limits<double>::max()}},
                 {{0, 100},
                  {200, 350},
                  {500, std::numeric_limits<double>::max()}},
             }}));
  }

  {
    auto expected = hisui::layout::TrimIntervals{
        .trim_intervals = {{250, 300},
                           {500, std::numeric_limits<double>::max()}},
    };
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::overlap_trim_intervals(
            {.list_of_trim_intervals = {
                 {{0, 100},
                  {200, 350},
                  {500, std::numeric_limits<double>::max()}},
                 {{200, 300}, {500, std::numeric_limits<double>::max()}},
                 {{250, 400}, {500, std::numeric_limits<double>::max()}},
             }}));
  }

  {
    auto expected = hisui::layout::TrimIntervals{
        .trim_intervals = {{0, 2},
                           {3, 5},
                           {500, std::numeric_limits<double>::max()}},
    };
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::overlap_trim_intervals(
            {.list_of_trim_intervals = {
                 {{0, 2}, {3, std::numeric_limits<double>::max()}},
                 {{0, 2}, {3, 5}, {500, std::numeric_limits<double>::max()}},
             }}));
  }
}

BOOST_AUTO_TEST_SUITE_END()
