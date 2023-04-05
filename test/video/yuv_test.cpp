#include <boost/test/unit_test.hpp>

#include "video/yuv.hpp"

BOOST_AUTO_TEST_SUITE(yuv)

BOOST_AUTO_TEST_CASE(merge_yuv_planes_from_top_left_1x2a) {
  const unsigned char p1[6] = {1, 1, 1, 1, 1, 1};
  const unsigned char p2[6] = {2, 2, 2, 2, 2, 2};
  std::vector<const unsigned char*> yuvs{p1, p2};
  unsigned char* merged = new unsigned char[12];

  hisui::video::merge_yuv_planes_from_top_left(merged, 12, 2, yuvs, 2, 3, 2, 0);
  unsigned char expected[12] = {1, 1, 1, 2, 2, 2, 1, 1, 1, 2, 2, 2};

  BOOST_REQUIRE_EQUAL_COLLECTIONS(expected, expected + 12, merged, merged + 12);

  delete[] merged;
}

BOOST_AUTO_TEST_CASE(merge_yuv_planes_from_top_left_1x2b) {
  const unsigned char p1[6] = {1, 1, 1, 1, 1, 1};
  const unsigned char p2[6] = {2, 2, 2, 2, 2, 2};
  std::vector<const unsigned char*> yuvs{p1, p2};
  unsigned char* merged = new unsigned char[12];

  hisui::video::merge_yuv_planes_from_top_left(merged, 12, 2, yuvs, 2, 2, 3, 0);
  unsigned char expected[12] = {1, 1, 2, 2, 1, 1, 2, 2, 1, 1, 2, 2};

  BOOST_REQUIRE_EQUAL_COLLECTIONS(expected, expected + 12, merged, merged + 12);

  delete[] merged;
}

BOOST_AUTO_TEST_CASE(merge_yuv_planes_from_top_left_2x1b) {
  const unsigned char p1[6] = {1, 1, 1, 1, 1, 1};
  const unsigned char p2[6] = {2, 2, 2, 2, 2, 2};
  std::vector<const unsigned char*> yuvs{p1, p2};

  unsigned char* merged = new unsigned char[12];
  hisui::video::merge_yuv_planes_from_top_left(merged, 12, 1, yuvs, 2, 3, 2, 0);
  unsigned char expected[12] = {1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2};

  BOOST_REQUIRE_EQUAL_COLLECTIONS(expected, expected + 12, merged, merged + 12);
  delete[] merged;
}

BOOST_AUTO_TEST_CASE(merge_yuv_planes_from_top_left_2x2a) {
  const unsigned char p1[6] = {1, 1, 1, 1, 1, 1};
  const unsigned char p2[6] = {2, 2, 2, 2, 2, 2};
  const unsigned char p3[6] = {3, 3, 3, 3, 3, 3};
  std::vector<const unsigned char*> yuvs{p1, p2, p3};

  unsigned char* merged = new unsigned char[24];
  hisui::video::merge_yuv_planes_from_top_left(merged, 24, 2, yuvs, 3, 3, 2, 0);
  unsigned char expected[24] = {1, 1, 1, 2, 2, 2, 1, 1, 1, 2, 2, 2,
                                3, 3, 3, 0, 0, 0, 3, 3, 3, 0, 0, 0};

  BOOST_REQUIRE_EQUAL_COLLECTIONS(expected, expected + 24, merged, merged + 24);
  delete[] merged;
}

BOOST_AUTO_TEST_CASE(merge_yuv_planes_from_top_left_2x2b) {
  unsigned char p1[6] = {1, 1, 1, 1, 1, 1};
  unsigned char p2[6] = {2, 2, 2, 2, 2, 2};
  unsigned char p3[6] = {3, 3, 3, 3, 3, 3};
  std::vector<const unsigned char*> yuvs{p1, p2, p3};

  unsigned char* merged = new unsigned char[24];
  hisui::video::merge_yuv_planes_from_top_left(merged, 24, 2, yuvs, 3, 3, 2,
                                               128);
  unsigned char expected[24] = {1, 1, 1, 2,   2,   2,   1, 1, 1, 2,   2,   2,
                                3, 3, 3, 128, 128, 128, 3, 3, 3, 128, 128, 128};

  BOOST_REQUIRE_EQUAL_COLLECTIONS(expected, expected + 24, merged, merged + 24);
  delete[] merged;
}

BOOST_AUTO_TEST_CASE(merge_yuv_planes_from_top_left_2x2c) {
  unsigned char p1[6] = {1, 1, 1, 1, 1, 1};
  unsigned char p2[6] = {2, 2, 2, 2, 2, 2};
  unsigned char p3[6] = {3, 3, 3, 3, 3, 3};
  unsigned char p4[6] = {4, 4, 4, 4, 4, 4};
  std::vector<const unsigned char*> yuvs{p1, p2, p3, p4};

  unsigned char* merged = new unsigned char[24];
  hisui::video::merge_yuv_planes_from_top_left(merged, 24, 2, yuvs, 4, 2, 3,
                                               128);
  unsigned char expected[24] = {1, 1, 2, 2, 1, 1, 2, 2, 1, 1, 2, 2,
                                3, 3, 4, 4, 3, 3, 4, 4, 3, 3, 4, 4};

  BOOST_REQUIRE_EQUAL_COLLECTIONS(expected, expected + 24, merged, merged + 24);
  delete[] merged;
}

BOOST_AUTO_TEST_CASE(create_black_yuv_image_1) {
  auto yuv = hisui::video::create_black_yuv_image(4, 2);

  BOOST_REQUIRE(yuv->checkWidthAndHeight(4, 2));
  BOOST_REQUIRE_EQUAL(4, yuv->getWidth(0));
  BOOST_REQUIRE_EQUAL(2, yuv->getWidth(1));
  BOOST_REQUIRE_EQUAL(2, yuv->getWidth(2));
  BOOST_REQUIRE_EQUAL(2, yuv->getHeight(0));
  BOOST_REQUIRE_EQUAL(1, yuv->getHeight(1));
  BOOST_REQUIRE_EQUAL(1, yuv->getHeight(2));

  unsigned char expected_y[] = {0, 0, 0, 0, 0, 0, 0, 0};
  unsigned char expected_u[] = {128, 128};
  unsigned char expected_v[] = {128, 128};

  BOOST_REQUIRE_EQUAL_COLLECTIONS(expected_y, expected_y + 8, yuv->yuv[0],
                                  yuv->yuv[0] + 8);
  BOOST_REQUIRE_EQUAL_COLLECTIONS(expected_u, expected_u + 2, yuv->yuv[1],
                                  yuv->yuv[1] + 2);
  BOOST_REQUIRE_EQUAL_COLLECTIONS(expected_v, expected_v + 2, yuv->yuv[2],
                                  yuv->yuv[2] + 2);
}

BOOST_AUTO_TEST_CASE(create_black_yuv_image_2) {
  auto yuv = hisui::video::create_black_yuv_image(3, 2);

  BOOST_REQUIRE(yuv->checkWidthAndHeight(3, 2));
  BOOST_REQUIRE_EQUAL(3, yuv->getWidth(0));
  BOOST_REQUIRE_EQUAL(2, yuv->getWidth(1));
  BOOST_REQUIRE_EQUAL(2, yuv->getWidth(2));
  BOOST_REQUIRE_EQUAL(2, yuv->getHeight(0));
  BOOST_REQUIRE_EQUAL(1, yuv->getHeight(1));
  BOOST_REQUIRE_EQUAL(1, yuv->getHeight(2));

  unsigned char expected_y[] = {
      0, 0, 0, 0, 0, 0,
  };
  unsigned char expected_u[] = {128, 128};
  unsigned char expected_v[] = {128, 128};

  BOOST_REQUIRE_EQUAL_COLLECTIONS(expected_y, expected_y + 6, yuv->yuv[0],
                                  yuv->yuv[0] + 6);
  BOOST_REQUIRE_EQUAL_COLLECTIONS(expected_u, expected_u + 2, yuv->yuv[1],
                                  yuv->yuv[1] + 2);
  BOOST_REQUIRE_EQUAL_COLLECTIONS(expected_v, expected_v + 2, yuv->yuv[2],
                                  yuv->yuv[2] + 2);
}

BOOST_AUTO_TEST_CASE(YUVIMage_setWidthAndHeight_1) {
  auto yuv = hisui::video::create_black_yuv_image(3, 2);

  BOOST_REQUIRE(yuv->checkWidthAndHeight(3, 2));
  BOOST_REQUIRE_EQUAL(3, yuv->getWidth(0));
  BOOST_REQUIRE_EQUAL(2, yuv->getWidth(1));
  BOOST_REQUIRE_EQUAL(2, yuv->getWidth(2));
  BOOST_REQUIRE_EQUAL(2, yuv->getHeight(0));
  BOOST_REQUIRE_EQUAL(1, yuv->getHeight(1));
  BOOST_REQUIRE_EQUAL(1, yuv->getHeight(2));

  yuv->setWidthAndHeight(6, 4);

  BOOST_REQUIRE(yuv->checkWidthAndHeight(6, 4));
  BOOST_REQUIRE_EQUAL(6, yuv->getWidth(0));
  BOOST_REQUIRE_EQUAL(3, yuv->getWidth(1));
  BOOST_REQUIRE_EQUAL(3, yuv->getWidth(2));
  BOOST_REQUIRE_EQUAL(4, yuv->getHeight(0));
  BOOST_REQUIRE_EQUAL(2, yuv->getHeight(1));
  BOOST_REQUIRE_EQUAL(2, yuv->getHeight(2));
}

BOOST_AUTO_TEST_SUITE_END()
