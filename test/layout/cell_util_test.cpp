#include <cstdint>
#include <stdexcept>

#include <boost/test/unit_test.hpp>

#include "layout/cell_util.hpp"

BOOST_AUTO_TEST_SUITE(cell)

BOOST_AUTO_TEST_CASE(calc_cell_length_and_positions) {
  BOOST_REQUIRE_THROW(
      hisui::layout::calc_cell_length_and_positions({.number_of_cells = 0}),
      std::invalid_argument);
  BOOST_REQUIRE_THROW(
      hisui::layout::calc_cell_length_and_positions(
          {.number_of_cells = 2, .region_length = 240, .min_frame_length = 1}),
      std::invalid_argument);
  {
    auto expected =
        hisui::layout::LengthAndPositions{.length = 236, .positions = {2}};
    BOOST_REQUIRE_EQUAL(expected, hisui::layout::calc_cell_length_and_positions(
                                      {.number_of_cells = 1,
                                       .region_length = 242,
                                       .min_frame_length = 2}));
  }
  {
    auto expected =
        hisui::layout::LengthAndPositions{.length = 240, .positions = {0}};
    BOOST_REQUIRE_EQUAL(expected, hisui::layout::calc_cell_length_and_positions(
                                      {.number_of_cells = 1,
                                       .region_length = 242,
                                       .min_frame_length = 2,
                                       .is_frame_on_ends = false}));
  }
  {
    auto expected =
        hisui::layout::LengthAndPositions{.length = 236, .positions = {2}};
    BOOST_REQUIRE_EQUAL(expected, hisui::layout::calc_cell_length_and_positions(
                                      {.number_of_cells = 1,
                                       .region_length = 240,
                                       .min_frame_length = 2}));
  }
  {
    auto expected =
        hisui::layout::LengthAndPositions{.length = 236, .positions = {2, 240}};
    BOOST_REQUIRE_EQUAL(expected, hisui::layout::calc_cell_length_and_positions(
                                      {.number_of_cells = 2,
                                       .region_length = 484,
                                       .min_frame_length = 2}));
  }
  {
    auto expected = hisui::layout::LengthAndPositions{
        .length = 156, .positions = {2, 160, 318}};
    BOOST_REQUIRE_EQUAL(expected, hisui::layout::calc_cell_length_and_positions(
                                      {.number_of_cells = 3,
                                       .region_length = 480,
                                       .min_frame_length = 2}));
  }
  {
    auto expected = hisui::layout::LengthAndPositions{
        .length = 156, .positions = {2, 160, 318}};
    BOOST_REQUIRE_EQUAL(expected, hisui::layout::calc_cell_length_and_positions(
                                      {.number_of_cells = 3,
                                       .region_length = 480,
                                       .min_frame_length = 2}));
  }
  {
    auto expected = hisui::layout::LengthAndPositions{
        .length = 156, .positions = {0, 158, 316}};
    BOOST_REQUIRE_EQUAL(expected, hisui::layout::calc_cell_length_and_positions(
                                      {.number_of_cells = 3,
                                       .region_length = 480,
                                       .min_frame_length = 2,
                                       .is_frame_on_ends = false}));
  }
}

BOOST_AUTO_TEST_CASE(calc_cell_resolution_and_positions) {
  BOOST_REQUIRE_THROW(hisui::layout::calc_cell_resolution_and_positions(
                          {.grid_dimension = {.columns = 0, .rows = 1}}),
                      std::invalid_argument);
  BOOST_REQUIRE_THROW(hisui::layout::calc_cell_resolution_and_positions(
                          {.grid_dimension = {.columns = 1, .rows = 0}}),
                      std::invalid_argument);

  {
    auto expected = hisui::layout::ResolutionAndPositions{
        .resolution = {.width = 236, .height = 156},
        .positions = {{.x = 2, .y = 2}}};
    BOOST_REQUIRE_EQUAL(expected,
                        hisui::layout::calc_cell_resolution_and_positions({
                            .grid_dimension = {.columns = 1, .rows = 1},
                            .region_resolution = {.width = 242, .height = 162},
                        }));
  }
  {
    auto expected = hisui::layout::ResolutionAndPositions{
        .resolution = {.width = 236, .height = 156},
        .positions = {{.x = 2, .y = 2}}};
    BOOST_REQUIRE_EQUAL(expected,
                        hisui::layout::calc_cell_resolution_and_positions({
                            .grid_dimension = {.columns = 1, .rows = 1},
                            .region_resolution = {.width = 242, .height = 162},
                            .min_frame_width = 2,
                            .min_frame_height = 2,
                        }));
  }
  {
    auto expected = hisui::layout::ResolutionAndPositions{
        .resolution = {.width = 116, .height = 76},
        .positions = {{.x = 2, .y = 2},
                      {.x = 120, .y = 2},
                      {.x = 2, .y = 80},
                      {.x = 120, .y = 80}}};
    BOOST_REQUIRE_EQUAL(expected,
                        hisui::layout::calc_cell_resolution_and_positions({
                            .grid_dimension = {.columns = 2, .rows = 2},
                            .region_resolution = {.width = 242, .height = 162},
                        }));
  }
}

BOOST_AUTO_TEST_SUITE_END()
