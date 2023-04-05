#include <boost/test/unit_test.hpp>

#include "util/wildcard.hpp"

BOOST_AUTO_TEST_SUITE(wildcard)

BOOST_AUTO_TEST_CASE(mix_sample_simple) {
  BOOST_REQUIRE(hisui::util::wildcard_match({.text = "", .pattern = "*"}));
  BOOST_REQUIRE(hisui::util::wildcard_match({.text = "aaa", .pattern = "*"}));
  BOOST_REQUIRE(hisui::util::wildcard_match({.text = "aaa", .pattern = "a*"}));
  BOOST_REQUIRE(hisui::util::wildcard_match({.text = "aaa", .pattern = "a**"}));
  BOOST_REQUIRE(hisui::util::wildcard_match({.text = "aaa", .pattern = "a*a"}));
  BOOST_REQUIRE(hisui::util::wildcard_match({.text = "aaa", .pattern = "*a"}));

  BOOST_REQUIRE(
      !hisui::util::wildcard_match({.text = "aaa", .pattern = "a*b"}));
  BOOST_REQUIRE(
      !hisui::util::wildcard_match({.text = "aaa", .pattern = "*ba"}));

  BOOST_REQUIRE(hisui::util::wildcard_match(
      {.text = "-adobe-courier-bold-o-normal--12-120-75-75-m-70-iso8859-1",
       .pattern = "-*-*-*-*-*-*-12-*-*-*-m-*-*-*"}));
  BOOST_REQUIRE(!hisui::util::wildcard_match(
      {.text = "-adobe-courier-bold-o-normal--12-120-75-75-X-70-iso8859-1",
       .pattern = "-*-*-*-*-*-*-12-*-*-*-m-*-*-*"}));
}

BOOST_AUTO_TEST_SUITE_END()
