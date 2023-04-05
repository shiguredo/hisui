#include <cstdint>
#include <stdexcept>

#include <boost/test/unit_test.hpp>

#include "layout/grid.hpp"

BOOST_AUTO_TEST_SUITE(grid)

BOOST_AUTO_TEST_CASE(calc_grid_dimension_unconstrained_grid) {
  BOOST_REQUIRE_THROW(
      hisui::layout::calc_grid_dimension({.number_of_sources = 0}),
      std::invalid_argument);

  {
    auto expected = hisui::layout::GridDimension{.columns = 1, .rows = 1};
    BOOST_REQUIRE_EQUAL(
        expected, hisui::layout::calc_grid_dimension({.number_of_sources = 1}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 2, .rows = 1};
    BOOST_REQUIRE_EQUAL(
        expected, hisui::layout::calc_grid_dimension({.number_of_sources = 2}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 2, .rows = 2};
    BOOST_REQUIRE_EQUAL(
        expected, hisui::layout::calc_grid_dimension({.number_of_sources = 3}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 2, .rows = 2};
    BOOST_REQUIRE_EQUAL(
        expected, hisui::layout::calc_grid_dimension({.number_of_sources = 4}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 3, .rows = 2};
    BOOST_REQUIRE_EQUAL(
        expected, hisui::layout::calc_grid_dimension({.number_of_sources = 5}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 3, .rows = 2};
    BOOST_REQUIRE_EQUAL(
        expected, hisui::layout::calc_grid_dimension({.number_of_sources = 6}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 3, .rows = 3};
    BOOST_REQUIRE_EQUAL(
        expected, hisui::layout::calc_grid_dimension({.number_of_sources = 7}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 3, .rows = 3};
    BOOST_REQUIRE_EQUAL(
        expected, hisui::layout::calc_grid_dimension({.number_of_sources = 9}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 4, .rows = 3};
    BOOST_REQUIRE_EQUAL(expected, hisui::layout::calc_grid_dimension(
                                      {.number_of_sources = 10}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 4, .rows = 3};
    BOOST_REQUIRE_EQUAL(expected, hisui::layout::calc_grid_dimension(
                                      {.number_of_sources = 12}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 5, .rows = 4};
    BOOST_REQUIRE_EQUAL(expected, hisui::layout::calc_grid_dimension(
                                      {.number_of_sources = 17}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 5, .rows = 4};
    BOOST_REQUIRE_EQUAL(expected, hisui::layout::calc_grid_dimension(
                                      {.number_of_sources = 20}));
  }
}

BOOST_AUTO_TEST_CASE(calc_grid_dimension_unconstrained_dimension_rows) {
  {
    auto expected = hisui::layout::GridDimension{.columns = 1, .rows = 1};
    BOOST_REQUIRE_EQUAL(expected,
                        hisui::layout::calc_grid_dimension(
                            {.max_columns = 1, .number_of_sources = 1}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 1, .rows = 2};
    BOOST_REQUIRE_EQUAL(expected,
                        hisui::layout::calc_grid_dimension(
                            {.max_columns = 1, .number_of_sources = 2}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 1, .rows = 3};
    BOOST_REQUIRE_EQUAL(expected,
                        hisui::layout::calc_grid_dimension(
                            {.max_columns = 1, .number_of_sources = 3}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 2, .rows = 2};
    BOOST_REQUIRE_EQUAL(expected,
                        hisui::layout::calc_grid_dimension(
                            {.max_columns = 2, .number_of_sources = 4}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 2, .rows = 3};
    BOOST_REQUIRE_EQUAL(expected,
                        hisui::layout::calc_grid_dimension(
                            {.max_columns = 2, .number_of_sources = 5}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 2, .rows = 4};
    BOOST_REQUIRE_EQUAL(expected,
                        hisui::layout::calc_grid_dimension(
                            {.max_columns = 2, .number_of_sources = 7}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 2, .rows = 5};
    BOOST_REQUIRE_EQUAL(expected,
                        hisui::layout::calc_grid_dimension(
                            {.max_columns = 2, .number_of_sources = 9}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 2, .rows = 6};
    BOOST_REQUIRE_EQUAL(expected,
                        hisui::layout::calc_grid_dimension(
                            {.max_columns = 2, .number_of_sources = 12}));
  }
}

BOOST_AUTO_TEST_CASE(calc_grid_dimension_unconstrained_dimension_columns) {
  {
    auto expected = hisui::layout::GridDimension{.columns = 1, .rows = 1};
    BOOST_REQUIRE_EQUAL(expected, hisui::layout::calc_grid_dimension(
                                      {.max_rows = 1, .number_of_sources = 1}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 2, .rows = 1};
    BOOST_REQUIRE_EQUAL(expected, hisui::layout::calc_grid_dimension(
                                      {.max_rows = 1, .number_of_sources = 2}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 3, .rows = 1};
    BOOST_REQUIRE_EQUAL(expected, hisui::layout::calc_grid_dimension(
                                      {.max_rows = 1, .number_of_sources = 3}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 2, .rows = 2};
    BOOST_REQUIRE_EQUAL(expected, hisui::layout::calc_grid_dimension(
                                      {.max_rows = 2, .number_of_sources = 4}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 3, .rows = 2};
    BOOST_REQUIRE_EQUAL(expected, hisui::layout::calc_grid_dimension(
                                      {.max_rows = 2, .number_of_sources = 5}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 4, .rows = 2};
    BOOST_REQUIRE_EQUAL(expected, hisui::layout::calc_grid_dimension(
                                      {.max_rows = 2, .number_of_sources = 7}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 5, .rows = 2};
    BOOST_REQUIRE_EQUAL(expected, hisui::layout::calc_grid_dimension(
                                      {.max_rows = 2, .number_of_sources = 9}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 6, .rows = 2};
    BOOST_REQUIRE_EQUAL(expected,
                        hisui::layout::calc_grid_dimension(
                            {.max_rows = 2, .number_of_sources = 12}));
  }
}

BOOST_AUTO_TEST_CASE(calc_grid_dimension_constrained_grid) {
  {
    auto expected = hisui::layout::GridDimension{.columns = 1, .rows = 1};
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::calc_grid_dimension(
            {.max_columns = 1, .max_rows = 1, .number_of_sources = 1}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 1, .rows = 1};
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::calc_grid_dimension(
            {.max_columns = 2, .max_rows = 1, .number_of_sources = 1}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 1, .rows = 1};
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::calc_grid_dimension(
            {.max_columns = 2, .max_rows = 2, .number_of_sources = 1}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 1, .rows = 1};
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::calc_grid_dimension(
            {.max_columns = 1, .max_rows = 1, .number_of_sources = 2}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 2, .rows = 1};
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::calc_grid_dimension(
            {.max_columns = 2, .max_rows = 1, .number_of_sources = 2}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 2, .rows = 1};
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::calc_grid_dimension(
            {.max_columns = 2, .max_rows = 2, .number_of_sources = 2}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 1, .rows = 1};
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::calc_grid_dimension(
            {.max_columns = 1, .max_rows = 1, .number_of_sources = 3}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 2, .rows = 1};
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::calc_grid_dimension(
            {.max_columns = 2, .max_rows = 1, .number_of_sources = 3}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 2, .rows = 2};
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::calc_grid_dimension(
            {.max_columns = 2, .max_rows = 2, .number_of_sources = 3}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 2, .rows = 2};
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::calc_grid_dimension(
            {.max_columns = 2, .max_rows = 2, .number_of_sources = 4}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 1, .rows = 1};
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::calc_grid_dimension(
            {.max_columns = 1, .max_rows = 1, .number_of_sources = 5}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 2, .rows = 1};
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::calc_grid_dimension(
            {.max_columns = 2, .max_rows = 1, .number_of_sources = 5}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 2, .rows = 2};
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::calc_grid_dimension(
            {.max_columns = 2, .max_rows = 2, .number_of_sources = 5}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 7, .rows = 1};
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::calc_grid_dimension(
            {.max_columns = 7, .max_rows = 1, .number_of_sources = 9}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 5, .rows = 2};
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::calc_grid_dimension(
            {.max_columns = 7, .max_rows = 2, .number_of_sources = 9}));
  }
  {
    auto expected = hisui::layout::GridDimension{.columns = 3, .rows = 3};
    BOOST_REQUIRE_EQUAL(
        expected,
        hisui::layout::calc_grid_dimension(
            {.max_columns = 7, .max_rows = 3, .number_of_sources = 9}));
  }
}

BOOST_AUTO_TEST_CASE(add_number_of_excluded_cells) {
  BOOST_REQUIRE_THROW(hisui::layout::add_number_of_excluded_cells(
                          {.number_of_sources = 1, .cells_excluded = {3, 1}}),
                      std::invalid_argument);

  BOOST_REQUIRE_EQUAL(0, hisui::layout::add_number_of_excluded_cells(
                             {.number_of_sources = 0, .cells_excluded = {}}));
  BOOST_REQUIRE_EQUAL(1, hisui::layout::add_number_of_excluded_cells(
                             {.number_of_sources = 1, .cells_excluded = {}}));
  BOOST_REQUIRE_EQUAL(2, hisui::layout::add_number_of_excluded_cells(
                             {.number_of_sources = 1, .cells_excluded = {0}}));
  BOOST_REQUIRE_EQUAL(1, hisui::layout::add_number_of_excluded_cells(
                             {.number_of_sources = 1, .cells_excluded = {1}}));
  BOOST_REQUIRE_EQUAL(1, hisui::layout::add_number_of_excluded_cells(
                             {.number_of_sources = 1, .cells_excluded = {1}}));

  BOOST_REQUIRE_EQUAL(3,
                      hisui::layout::add_number_of_excluded_cells(
                          {.number_of_sources = 2, .cells_excluded = {1, 3}}));
  BOOST_REQUIRE_EQUAL(5,
                      hisui::layout::add_number_of_excluded_cells(
                          {.number_of_sources = 3, .cells_excluded = {1, 3}}));
}

BOOST_AUTO_TEST_SUITE_END()
